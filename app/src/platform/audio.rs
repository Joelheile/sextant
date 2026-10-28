//! Call audio on macOS (plan.md §4, §12): which processes have the mic running (Core Audio
//! process objects), the mic itself (cpal), and a tap on the call app's own sound (a Core Audio
//! process tap in a private aggregate device, macOS 14.2+). Nothing here records anything by
//! itself: a stream delivers blocks into an inbox only while the pipeline holds it open, and
//! dropping the stream stops it.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::{self, NonNull};

use anyhow::{Context, Result, bail};
use block2::RcBlock;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use objc2::rc::Retained;
use objc2::{AllocAnyThread, Message};
use objc2_app_kit::NSRunningApplication;
use objc2_core_audio::{
    AudioDeviceCreateIOProcIDWithBlock, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectID, AudioObjectPropertyAddress, CATapDescription, CATapMuteBehavior,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceMainSubDeviceKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceUID,
    kAudioHardwarePropertyDefaultSystemOutputDevice, kAudioHardwarePropertyProcessObjectList,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
    kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyPID,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSString, NSUUID};
use sextant_core::audio::MicUser;

use crate::audio::{Chunk, Devices, Inbox};

pub struct Mac;

/// A running stream. Dropping it stops it.
pub enum Stream {
    Mic(#[allow(dead_code)] cpal::Stream),
    Tap(#[allow(dead_code)] Tap),
}

// SAFETY: a stream is created, kept and dropped on the audio thread and never touched from
// another one. The type only has to be `Send` because the thread's state moves into the thread
// once, with no stream in it yet.
unsafe impl Send for Stream {}

impl Devices for Mac {
    type Stream = Stream;

    fn mic_users(&mut self) -> Vec<MicUser> {
        list(
            kAudioObjectSystemObject as AudioObjectID,
            kAudioHardwarePropertyProcessObjectList,
        )
        .into_iter()
        .filter_map(|process| {
            let running: u32 = scalar(process, kAudioProcessPropertyIsRunningInput)?;
            if running == 0 {
                return None;
            }
            let pid: i32 = scalar(process, kAudioProcessPropertyPID)?;
            let bundle_id = string(process, kAudioProcessPropertyBundleID).unwrap_or_default();
            let name = app_name(pid).unwrap_or_else(|| bundle_id.clone());
            Some(MicUser {
                pid,
                bundle_id,
                name,
            })
        })
        .collect()
    }

    fn open_mic(&mut self, into: Inbox) -> Result<Stream> {
        let device = cpal::default_host()
            .default_input_device()
            .context("no microphone")?;
        let supported = device.default_input_config().context("microphone format")?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            bail!("microphone does not deliver f32 samples");
        }
        let (channels, rate) = (supported.channels(), supported.sample_rate());
        let stream = device
            .build_input_stream(
                supported.config(),
                move |samples: &[f32], _| {
                    into.force_push(Chunk {
                        samples: samples.to_vec(),
                        channels,
                        rate,
                    });
                },
                |_| {},
                None,
            )
            .context("open the microphone")?;
        stream.play().context("start the microphone")?;
        Ok(Stream::Mic(stream))
    }

    fn open_system(&mut self, pids: &[i32], into: Inbox) -> Result<Stream> {
        Tap::open(pids, into).map(Stream::Tap)
    }
}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// A fixed-size property such as a pid or a flag.
fn scalar<T: Copy + Default>(object: AudioObjectID, selector: u32) -> Option<T> {
    qualified(object, selector, ptr::null(), 0)
}

fn qualified<T: Copy + Default>(
    object: AudioObjectID,
    selector: u32,
    qualifier: *const c_void,
    qualifier_size: u32,
) -> Option<T> {
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    let mut at = address(selector);
    // SAFETY: the pointers are to live locals and `size` is the size of `value`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut at),
            qualifier_size,
            qualifier,
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    (status == 0).then_some(value)
}

fn list(object: AudioObjectID, selector: u32) -> Vec<AudioObjectID> {
    let mut at = address(selector);
    let mut size = 0u32;
    // SAFETY: as in `qualified`.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&mut at),
            0,
            ptr::null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
    // SAFETY: `ids` has room for `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut at),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(ids.as_mut_ptr().cast()).expect("vec pointer"),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / size_of::<AudioObjectID>());
    ids
}

fn string(object: AudioObjectID, selector: u32) -> Option<String> {
    let raw: usize = scalar(object, selector)?;
    // SAFETY: Core Audio returns a +1 CFString for these properties, or null.
    let string = unsafe { CFRetained::from_raw(NonNull::new(raw as *mut CFString)?) };
    Some(string.to_string())
}

fn app_name(pid: i32) -> Option<String> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    Some(app.localizedName()?.to_string())
}

fn process_object(pid: i32) -> Option<AudioObjectID> {
    let object: AudioObjectID = qualified(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyTranslatePIDToProcessObject,
        ptr::from_ref(&pid).cast(),
        size_of::<i32>() as u32,
    )?;
    (object != 0).then_some(object)
}

type IoBlock = dyn Fn(
    NonNull<objc2_core_audio_types::AudioTimeStamp>,
    NonNull<AudioBufferList>,
    NonNull<objc2_core_audio_types::AudioTimeStamp>,
    NonNull<AudioBufferList>,
    NonNull<objc2_core_audio_types::AudioTimeStamp>,
);

/// A tap on the sound the call app plays, read through a private aggregate device.
pub struct Tap {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    started: bool,
    _block: Option<RcBlock<IoBlock>>,
}

/// Every value handed to Core Foundation is an Objective-C object.
fn object<T: Message>(value: Retained<T>) -> Retained<NSObject> {
    // SAFETY: only NSString, NSNumber, NSArray and NSDictionary pass through here.
    unsafe { Retained::cast_unchecked(value) }
}

fn dictionary(pairs: &[(&str, Retained<NSObject>)]) -> Retained<NSDictionary<NSString, NSObject>> {
    let keys: Vec<Retained<NSString>> = pairs
        .iter()
        .map(|(key, _)| NSString::from_str(key))
        .collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
    let values: Vec<Retained<NSObject>> = pairs.iter().map(|(_, value)| value.clone()).collect();
    NSDictionary::from_retained_objects(&key_refs, &values)
}

impl Tap {
    fn open(pids: &[i32], into: Inbox) -> Result<Self> {
        let processes: Vec<Retained<NSNumber>> = pids
            .iter()
            .filter_map(|pid| process_object(*pid))
            .map(NSNumber::new_u32)
            .collect();
        if processes.is_empty() {
            bail!("the call app has no audio process");
        }
        let array = NSArray::from_retained_slice(&processes);
        // SAFETY: plain Objective-C calls on objects created here.
        let description = unsafe {
            CATapDescription::initStereoMixdownOfProcesses(CATapDescription::alloc(), &array)
        };
        let uuid = unsafe { description.UUID().UUIDString() };
        unsafe {
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }
        let mut tap = 0;
        // SAFETY: `tap` is a valid out pointer.
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap) };
        if status != 0 {
            bail!("the process tap was refused ({status})");
        }
        let mut this = Self {
            tap,
            aggregate: 0,
            proc_id: None,
            started: false,
            _block: None,
        };
        this.attach(&uuid, into)?;
        Ok(this)
    }

    /// Wraps the tap in a private aggregate device, reads its format and starts the callback.
    fn attach(&mut self, tap_uid: &NSString, into: Inbox) -> Result<()> {
        let format: AudioStreamBasicDescription =
            scalar_format(self.tap).context("the tap has no format")?;
        let output = default_output_uid().context("no output device")?;
        let sub_tap = dictionary(&[
            (
                &*kAudioSubTapUIDKey.to_string_lossy(),
                object(tap_uid.retain()),
            ),
            (
                &*kAudioSubTapDriftCompensationKey.to_string_lossy(),
                object(NSNumber::new_bool(true)),
            ),
        ]);
        let aggregate_uid = NSUUID::UUID().UUIDString();
        let description = dictionary(&[
            (
                &*kAudioAggregateDeviceUIDKey.to_string_lossy(),
                object(aggregate_uid),
            ),
            (
                &*kAudioAggregateDeviceNameKey.to_string_lossy(),
                object(NSString::from_str("Sextant call tap")),
            ),
            (
                &*kAudioAggregateDeviceMainSubDeviceKey.to_string_lossy(),
                object(output),
            ),
            (
                &*kAudioAggregateDeviceIsPrivateKey.to_string_lossy(),
                object(NSNumber::new_bool(true)),
            ),
            (
                &*kAudioAggregateDeviceTapAutoStartKey.to_string_lossy(),
                object(NSNumber::new_bool(true)),
            ),
            (
                &*kAudioAggregateDeviceTapListKey.to_string_lossy(),
                object(NSArray::from_retained_slice(&[sub_tap])),
            ),
        ]);
        // SAFETY: an NSDictionary is a CFDictionary (toll-free bridged).
        let cf = unsafe { &*Retained::as_ptr(&description).cast::<CFDictionary>() };
        let mut aggregate = 0;
        // SAFETY: `aggregate` is a valid out pointer.
        let status =
            unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut aggregate)) };
        if status != 0 {
            bail!("the aggregate device was refused ({status})");
        }
        self.aggregate = aggregate;

        let (rate, channels) = (format.mSampleRate as u32, format.mChannelsPerFrame as u16);
        let block = RcBlock::new(
            move |_: NonNull<_>,
                  input: NonNull<AudioBufferList>,
                  _: NonNull<_>,
                  _: NonNull<_>,
                  _: NonNull<_>| {
                // SAFETY: the list is valid for the length of the callback.
                if let Some(samples) = unsafe { interleaved(input) } {
                    into.force_push(Chunk {
                        samples,
                        channels,
                        rate,
                    });
                }
            },
        );
        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: the block lives in `self` until the proc id is destroyed in `drop`.
        let status = unsafe {
            AudioDeviceCreateIOProcIDWithBlock(
                NonNull::from(&mut proc_id),
                self.aggregate,
                None,
                RcBlock::as_ptr(&block),
            )
        };
        if status != 0 {
            bail!("the tap callback was refused ({status})");
        }
        self.proc_id = proc_id;
        self._block = Some(block);
        // SAFETY: device and proc id were just created.
        let status = unsafe { AudioDeviceStart(self.aggregate, self.proc_id) };
        if status != 0 {
            bail!("the tap did not start ({status})");
        }
        self.started = true;
        Ok(())
    }
}

fn scalar_format(tap: AudioObjectID) -> Option<AudioStreamBasicDescription> {
    let mut value = AudioStreamBasicDescription {
        mSampleRate: 0.0,
        mFormatID: 0,
        mFormatFlags: 0,
        mBytesPerPacket: 0,
        mFramesPerPacket: 0,
        mBytesPerFrame: 0,
        mChannelsPerFrame: 0,
        mBitsPerChannel: 0,
        mReserved: 0,
    };
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    let mut at = address(kAudioTapPropertyFormat);
    // SAFETY: as in `qualified`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap,
            NonNull::from(&mut at),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    (status == 0 && value.mSampleRate > 0.0 && value.mChannelsPerFrame > 0).then_some(value)
}

fn default_output_uid() -> Option<Retained<NSString>> {
    let device: AudioObjectID = scalar(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyDefaultSystemOutputDevice,
    )?;
    string(device, kAudioDevicePropertyDeviceUID).map(|uid| NSString::from_str(&uid))
}

/// The samples of a buffer list as one interleaved f32 block. One buffer is taken as it is; one
/// buffer per channel is interleaved.
unsafe fn interleaved(list: NonNull<AudioBufferList>) -> Option<Vec<f32>> {
    // SAFETY: `mBuffers` holds `mNumberBuffers` entries.
    let list = unsafe { list.as_ref() };
    let buffers =
        unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize) };
    let view = |buffer: &objc2_core_audio_types::AudioBuffer| -> Option<&[f32]> {
        let data = NonNull::new(buffer.mData)?.cast::<f32>();
        // SAFETY: the buffer holds `mDataByteSize` bytes of f32 samples.
        Some(unsafe {
            std::slice::from_raw_parts(data.as_ptr(), buffer.mDataByteSize as usize / 4)
        })
    };
    match buffers {
        [] => None,
        [one] => view(one).map(<[f32]>::to_vec),
        many => {
            let planes: Vec<&[f32]> = many.iter().filter_map(view).collect();
            let frames = planes.iter().map(|plane| plane.len()).min()?;
            Some(
                (0..frames)
                    .flat_map(|frame| planes.iter().map(move |plane| plane[frame]))
                    .collect(),
            )
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // SAFETY: each id is destroyed once, in reverse order of creation, and only if it exists.
        unsafe {
            if self.started {
                AudioDeviceStop(self.aggregate, self.proc_id);
            }
            if self.proc_id.is_some() {
                AudioDeviceDestroyIOProcID(self.aggregate, self.proc_id);
            }
            if self.aggregate != 0 {
                AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            AudioHardwareDestroyProcessTap(self.tap);
        }
    }
}
