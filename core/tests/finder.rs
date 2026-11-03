use sextant_core::Settings;
use sextant_core::fixtures::*;

#[test]
fn refresh_is_deterministic_and_repeatable() {
    let settings = Settings::default();
    let (_a, mut first) = refreshed("determinism-a");
    let (_b, second) = refreshed("determinism-b");
    let week = |store: &sextant_core::Store| {
        store
            .week_page(&settings, WEEK_START, UTC_OFFSET_S)
            .expect("test setup")
    };
    let before = week(&first);
    assert_eq!(before, week(&second));
    first.refresh(local(WEEK_NOW)).expect("test setup");
    assert_eq!(before, week(&first));
}

#[test]
fn only_the_last_90_days_count() {
    let (_db, mut store) = refreshed("window");
    let later = WEEK_NOW + 100 * 86_400;
    let stats = store.refresh(local(later)).expect("test setup");
    assert_eq!(stats.episodes, 0);
    assert_eq!(stats.tasks, 0);
}
