//! The consented model download. This is the only file in Sextant that talks to
//! the network, and only after the user pressed "Download": pinned URLs, SHA-256 checked, written
//! into `<data dir>/models` by an atomic rename. `scripts/check-network.sh` keeps it that way.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// One file the app needs: where it comes from, what it must hash to, where it ends up.
pub struct Asset {
    /// Path below the models folder.
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// Everything speech to text needs: the voice activity model and NVIDIA Parakeet TDT 0.6B v3
/// (CC-BY-4.0), int8. ONNX Runtime ships inside the app.
pub const ASSETS: &[Asset] = &[
    Asset {
        file: "vad/silero_vad.onnx",
        url: "https://raw.githubusercontent.com/snakers4/silero-vad/v6.2.3/src/silero_vad/data/silero_vad.onnx",
        sha256: "1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3",
        bytes: 2_327_524,
    },
    Asset {
        file: "parakeet/vocab.txt",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce/vocab.txt",
        sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
        bytes: 93_939,
    },
    Asset {
        file: "parakeet/decoder_joint-model.int8.onnx",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce/decoder_joint-model.int8.onnx",
        sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
        bytes: 18_202_004,
    },
    Asset {
        file: "parakeet/encoder-model.int8.onnx",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce/encoder-model.int8.onnx",
        sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
        bytes: 652_183_999,
    },
];

/// Download progress over all assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

pub fn total_bytes(assets: &[Asset]) -> u64 {
    assets.iter().map(|asset| asset.bytes).sum()
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

/// Whether every asset is in place at its expected size. Hashes were checked at install time and
/// the install is an atomic rename, so a file of the right size is a whole one.
pub fn installed(models: &Path, assets: &[Asset]) -> bool {
    assets.iter().all(|asset| {
        fs::metadata(models.join(asset.file)).is_ok_and(|meta| meta.len() == asset.bytes)
    })
}

/// How a URL becomes a file. The real one is `UrlSession`; tests bring their own.
pub trait Fetch {
    /// Downloads `url` to `dest`. Calls `progress` with the bytes received so far, and gives up
    /// with an error soon after `cancel` is set.
    fn fetch(
        &self,
        url: &str,
        dest: &Path,
        progress: &dyn Fn(u64),
        cancel: &AtomicBool,
    ) -> Result<()>;
}

/// Fetches whatever is missing. Safe to call again after a failure or a cancel: finished assets
/// are skipped and half-finished ones start over.
pub fn download(
    models: &Path,
    assets: &[Asset],
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_progress: &dyn Fn(Progress),
) -> Result<()> {
    let total = total_bytes(assets);
    let partial = models.join(".partial");
    let mut done = 0;
    for asset in assets {
        let target = models.join(asset.file);
        if fs::metadata(&target).is_ok_and(|meta| meta.len() == asset.bytes) {
            done += asset.bytes;
            on_progress(Progress { done, total });
            continue;
        }
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(&partial).context("create the models folder")?;
        let file = partial.join("download");
        let base = done;
        fetch.fetch(
            asset.url,
            &file,
            &|received| {
                on_progress(Progress {
                    done: base + received.min(asset.bytes),
                    total,
                });
            },
            cancel,
        )?;
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        verify(&file, asset.sha256, asset.bytes).with_context(|| asset.file.to_string())?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).context("create the models folder")?;
        }
        fs::rename(&file, &target).context("install a model file")?;
        done += asset.bytes;
        on_progress(Progress { done, total });
    }
    let _ = fs::remove_dir_all(&partial);
    Ok(())
}

fn verify(path: &Path, sha256: &str, bytes: u64) -> Result<()> {
    let mut file = File::open(path).context("open the download")?;
    let mut hash = Sha256::new();
    let mut chunk = vec![0; 1 << 20];
    let mut len = 0;
    loop {
        let read = file.read(&mut chunk).context("read the download")?;
        if read == 0 {
            break;
        }
        hash.update(&chunk[..read]);
        len += read as u64;
    }
    if len != bytes {
        bail!("download has {len} bytes, expected {bytes}");
    }
    let hex: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if hex != sha256 {
        bail!("download does not match its checksum");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod urlsession {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use anyhow::{Result, anyhow, bail};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2_foundation::{
        NSError, NSHTTPURLResponse, NSString, NSURL, NSURLResponse, NSURLSession,
        NSURLSessionConfiguration,
    };

    use super::Fetch;

    /// Downloads with the system's URL loading, so no network crate is linked.
    pub struct UrlSession;

    impl Fetch for UrlSession {
        fn fetch(
            &self,
            url: &str,
            dest: &Path,
            progress: &dyn Fn(u64),
            cancel: &AtomicBool,
        ) -> Result<()> {
            let url = NSURL::URLWithString(&NSString::from_str(url))
                .ok_or_else(|| anyhow!("not a URL: {url}"))?;
            let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
            config.setTimeoutIntervalForRequest(60.0);
            let session = NSURLSession::sessionWithConfiguration(&config);
            let (done, finished) = mpsc::channel();
            let dest = dest.to_path_buf();
            let handler = RcBlock::new(
                move |location: *mut NSURL, response: *mut NSURLResponse, error: *mut NSError| {
                    // SAFETY: the pointers are valid for the length of this call.
                    let result = unsafe { finish(location, response, error, &dest) };
                    let _ = done.send(result);
                },
            );
            // SAFETY: the handler matches the completion handler signature.
            let task = unsafe { session.downloadTaskWithURL_completionHandler(&url, &handler) };
            task.resume();
            loop {
                match finished.recv_timeout(Duration::from_millis(200)) {
                    Ok(result) => return result.map_err(|message| anyhow!(message)),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if cancel.load(Ordering::Relaxed) {
                            task.cancel();
                        }
                        progress(task.countOfBytesReceived().max(0) as u64);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => bail!("download ended early"),
                }
            }
        }
    }

    /// Moves the finished download to `dest`, or says why there is none.
    unsafe fn finish(
        location: *mut NSURL,
        response: *mut NSURLResponse,
        error: *mut NSError,
        dest: &Path,
    ) -> Result<(), String> {
        if let Some(error) = unsafe { error.as_ref() } {
            return Err(error.localizedDescription().to_string());
        }
        // SAFETY: a null response is turned into None by `as_ref`.
        let status = unsafe { response.as_ref() }
            .and_then(|response| response.downcast_ref::<NSHTTPURLResponse>())
            .map(|response| response.statusCode());
        if status != Some(200) {
            return Err(format!("the server answered {status:?}"));
        }
        let location = unsafe { location.as_ref() }
            .and_then(NSURL::path)
            .map(|path: Retained<NSString>| path.to_string())
            .ok_or("no downloaded file")?;
        std::fs::rename(&location, dest)
            .or_else(|_| std::fs::copy(&location, dest).map(|_| ()))
            .map_err(|error| error.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
mod urlsession {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use anyhow::{Result, bail};

    use super::Fetch;

    pub struct UrlSession;

    impl Fetch for UrlSession {
        fn fetch(&self, _: &str, _: &Path, _: &dyn Fn(u64), _: &AtomicBool) -> Result<()> {
            bail!("downloading the models needs macOS")
        }
    }
}

pub use urlsession::UrlSession;

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const WORLD: &str = "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7";

    struct Fake {
        files: HashMap<&'static str, Vec<u8>>,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(files: &[(&'static str, &[u8])]) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(url, bytes)| (*url, bytes.to_vec()))
                    .collect(),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Fetch for Fake {
        fn fetch(
            &self,
            url: &str,
            dest: &Path,
            progress: &dyn Fn(u64),
            _: &AtomicBool,
        ) -> Result<()> {
            self.calls.borrow_mut().push(url.to_string());
            let bytes = self.files.get(url).context("no such url")?;
            progress(bytes.len() as u64 / 2);
            fs::write(dest, bytes)?;
            Ok(())
        }
    }

    fn assets() -> [Asset; 2] {
        [
            Asset {
                file: "a/hello.txt",
                url: "u://hello",
                sha256: HELLO,
                bytes: 5,
            },
            Asset {
                file: "b/world.txt",
                url: "u://world",
                sha256: WORLD,
                bytes: 5,
            },
        ]
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sextant-models-{}", uuid_like()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn uuid_like() -> String {
        use std::sync::atomic::AtomicU32;
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn quiet(_: Progress) {}

    #[test]
    fn downloads_verifies_and_installs_every_asset() {
        let models = temp();
        let fake = Fake::new(&[("u://hello", b"hello"), ("u://world", b"world")]);
        let seen = RefCell::new(Vec::new());
        download(
            &models,
            &assets(),
            &fake,
            &AtomicBool::new(false),
            &|progress| seen.borrow_mut().push(progress),
        )
        .expect("download");
        assert!(installed(&models, &assets()));
        assert_eq!(
            fs::read(models.join("b/world.txt")).expect("file"),
            b"world"
        );
        assert!(!models.join(".partial").exists());
        let seen = seen.into_inner();
        assert_eq!(
            seen.last(),
            Some(&Progress {
                done: 10,
                total: 10
            })
        );
        assert!(seen.windows(2).all(|pair| pair[0].done <= pair[1].done));
        let _ = fs::remove_dir_all(models);
    }

    #[test]
    fn a_wrong_checksum_installs_nothing() {
        let models = temp();
        let fake = Fake::new(&[("u://hello", b"HELLO"), ("u://world", b"world")]);
        let error = download(&models, &assets(), &fake, &AtomicBool::new(false), &quiet)
            .expect_err("must fail");
        assert!(format!("{error:#}").contains("checksum"), "{error:#}");
        assert!(!models.join("a/hello.txt").exists());
        assert!(!installed(&models, &assets()));
        let _ = fs::remove_dir_all(models);
    }

    #[test]
    fn a_wrong_size_installs_nothing() {
        let models = temp();
        let fake = Fake::new(&[("u://hello", b"hello!"), ("u://world", b"world")]);
        assert!(download(&models, &assets(), &fake, &AtomicBool::new(false), &quiet).is_err());
        assert!(!models.join("a/hello.txt").exists());
        let _ = fs::remove_dir_all(models);
    }

    #[test]
    fn a_second_run_only_fetches_what_is_missing() {
        let models = temp();
        let fake = Fake::new(&[("u://hello", b"hello"), ("u://world", b"world")]);
        let cancel = AtomicBool::new(false);
        download(&models, &assets(), &fake, &cancel, &quiet).expect("first");
        fs::remove_file(models.join("b/world.txt")).expect("remove");
        assert!(!installed(&models, &assets()));
        fake.calls.borrow_mut().clear();
        download(&models, &assets(), &fake, &cancel, &quiet).expect("second");
        assert_eq!(*fake.calls.borrow(), ["u://world"]);
        assert!(installed(&models, &assets()));
        let _ = fs::remove_dir_all(models);
    }

    #[test]
    fn a_cancel_stops_before_installing() {
        let models = temp();
        let fake = Fake::new(&[("u://hello", b"hello"), ("u://world", b"world")]);
        let error = download(&models, &assets(), &fake, &AtomicBool::new(true), &quiet)
            .expect_err("must stop");
        assert!(error.to_string().contains("cancelled"));
        assert!(!models.join("a/hello.txt").exists());
        let _ = fs::remove_dir_all(models);
    }

    #[test]
    fn the_pinned_assets_are_well_formed() {
        for asset in ASSETS {
            assert!(asset.url.starts_with("https://"), "{}", asset.url);
            assert_eq!(asset.sha256.len(), 64, "{}", asset.file);
        }
        assert!(total_bytes(ASSETS) < 800 * 1024 * 1024);
    }
}
