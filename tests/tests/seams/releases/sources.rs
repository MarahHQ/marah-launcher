//! The sources list: bundled, fetched, and the forums it names.

use super::*;

const SOURCES_URL: &str = "https://xyz.dev/catalog-sources.json";

/// A catalog sources file with these forums for the PS5, by key and address.
fn sources_of(forums: &[(&str, &str)]) -> String {
    let forums: Vec<serde_json::Value> =
        forums.iter().map(|(key, url)| serde_json::json!({ "key": key, "url": url, "tag": "PS5" })).collect();
    serde_json::json!({ "ps5": { "platform": "PS5", "forums": forums } }).to_string()
}

async fn set_sources(t: &TestCore, url: &str) -> launcher_core::CoreResult<()> {
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "sources_url": url }),
        })
        .await
}

/// The rows of the catalog plugin's "Catalog sources" section, as `label: value`.
async fn sources_rows(t: &TestCore) -> Vec<String> {
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    let request = launcher_api::SlotRequest { slot: launcher_api::SlotId::EmulatorPanel, game: None };
    let items = provided.slots.unwrap().items(&request).await.unwrap();
    let section = items.into_iter().find(|i| i.id == "sources").expect("a sources section");
    let launcher_api::SlotContent::Section { title, rows } = section.content else { panic!("a section") };
    assert_eq!(title, "Catalog sources");
    rows.into_iter().map(|r| format!("{}: {}", r.label, r.value)).collect()
}

/// The "Catalog sources" rows once `done` says they are the ones the test waits for, or after a
/// second of the clock: the Game Catalog fetches the sources in its own task, and a read gets
/// the copy kept now.
async fn rows_once(t: &TestCore, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    for _ in 0..100 {
        if done(&sources_rows(t).await) {
            break;
        }
        t.clock.advance(std::time::Duration::from_millis(10)).await;
    }
    sources_rows(t).await
}

/// Makes the kept sources copy six hours old.
fn age_the_kept_sources(t: &TestCore) {
    let kept = t.data_dir().join("plugins/release-catalog/sources/catalog-sources.meta.json");
    let mut stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&kept).unwrap()).unwrap();
    stored["fetched"] = 0.into();
    std::fs::write(&kept, stored.to_string()).unwrap();
}

#[tokio::test]
async fn without_a_sources_address_the_bundled_sources_are_used() {
    let t = boot().await;
    let rows = sources_rows(&t).await;
    assert_eq!(rows[0], "List: the bundled list");
    assert!(rows.contains(&"PS5: ps5".to_owned()), "{rows:?}");
    assert!(rows.contains(&"PC: linux, wine".to_owned()), "{rows:?}");
    assert!(t.http.sent().is_empty(), "nothing is fetched");
}

#[tokio::test]
async fn a_fetched_sources_list_is_used_and_kept_for_when_the_fetch_fails() {
    let t = boot().await;
    t.http.serve(SOURCES_URL, sources_of(&[("new", "https://xyz.net/forum/viewforum.php?f=1")]));
    set_sources(&t, SOURCES_URL).await.unwrap();

    // The first read gets the bundled list; the fetch runs in the background.
    let fetched = rows_once(&t, |r| r[0] == "List: fetched from xyz.dev").await;
    assert_eq!(fetched, ["List: fetched from xyz.dev", "PS5: new"]);
    assert!(t.data_dir().join("plugins/release-catalog/sources/catalog-sources.json").is_file());
    // A kept copy is used for six hours, with no new request.
    sources_rows(&t).await;
    t.clock.run_until_idle().await;
    assert_eq!(t.http.sent(), [SOURCES_URL]);

    // Six hours on, the address answers with a file that doesn't read: the kept copy stays.
    age_the_kept_sources(&t);
    t.http.serve(SOURCES_URL, b"{ not json".to_vec());
    let rows = rows_once(&t, |r| r[0] != "List: fetched from xyz.dev").await;
    assert_eq!(rows[0], "List: the copy fetched before from xyz.dev");
    assert!(
        rows[1].starts_with("Fetch failed: this is not a catalog sources file in the current format: "),
        "{rows:?}"
    );
    assert_eq!(rows[2], "PS5: new");
}

#[tokio::test]
async fn with_no_sources_copy_kept_a_failed_fetch_falls_back_to_the_bundled_sources() {
    let t = boot().await;
    set_sources(&t, SOURCES_URL).await.unwrap();
    let rows = rows_once(&t, |r| r.iter().any(|row| row.starts_with("Fetch failed"))).await;
    assert_eq!(rows[0], "List: the bundled list");
    assert_eq!(rows[1], format!("Fetch failed: HTTP 404 from {SOURCES_URL}"));
    assert!(rows.contains(&"PS5: ps5".to_owned()), "{rows:?}");
}

#[tokio::test]
async fn a_read_with_a_stale_sources_copy_answers_at_once_from_it_and_the_fetch_runs_after() {
    let t = boot().await;
    t.http.serve(SOURCES_URL, sources_of(&[("old", "https://xyz.net/forum/viewforum.php?f=1")]));
    set_sources(&t, SOURCES_URL).await.unwrap();
    rows_once(&t, |r| r[0] == "List: fetched from xyz.dev").await;

    // Six hours on, the address answers only when the test lets it.
    age_the_kept_sources(&t);
    let gate = t.http.hold_next(SOURCES_URL);
    t.http.serve(SOURCES_URL, sources_of(&[("new", "https://xyz.net/forum/viewforum.php?f=2")]));
    let read = tokio::time::timeout(std::time::Duration::from_millis(200), sources_rows(&t)).await;
    assert_eq!(read.expect("an answer at once"), ["List: fetched from xyz.dev", "PS5: old"], "the kept copy");

    // The fetch ends in the plugin's own task, and the next read gets the new copy.
    gate.release();
    assert_eq!(rows_once(&t, |r| r.contains(&"PS5: new".to_owned())).await, ["List: fetched from xyz.dev", "PS5: new"]);
    assert_eq!(t.http.sent(), [SOURCES_URL, SOURCES_URL]);
}

#[tokio::test]
async fn a_failed_sources_fetch_waits_before_it_tries_again_and_a_read_does_not_make_it_sooner() {
    let t = boot().await;
    set_sources(&t, SOURCES_URL).await.unwrap();
    rows_once(&t, |r| r.iter().any(|row| row.starts_with("Fetch failed"))).await;
    assert_eq!(t.http.sent(), [SOURCES_URL]);

    // More reads, with the clock short of the first wait: no new try.
    for _ in 0..3 {
        assert_eq!(sources_rows(&t).await[1], format!("Fetch failed: HTTP 404 from {SOURCES_URL}"));
        t.clock.advance(std::time::Duration::from_secs(60)).await;
    }
    assert_eq!(t.http.sent(), [SOURCES_URL], "the failed fetch is held");

    // Past the first wait, the plugin's task tries again with no read.
    t.clock.advance(std::time::Duration::from_secs(release_catalog::RETRY)).await;
    assert_eq!(t.http.sent(), [SOURCES_URL, SOURCES_URL]);
}

#[tokio::test]
async fn a_forum_with_no_f_number_is_skipped_and_named() {
    let t = boot().await;
    t.http.serve(
        SOURCES_URL,
        sources_of(&[("ok", "https://xyz.net/forum/viewforum.php?f=1"), ("bad", "https://xyz.net/forum/index.php")]),
    );
    set_sources(&t, SOURCES_URL).await.unwrap();
    assert_eq!(
        rows_once(&t, |r| r[0] == "List: fetched from xyz.dev").await,
        ["List: fetched from xyz.dev", "PS5: ok", "Skipped: No f= number: bad: https://xyz.net/forum/index.php"]
    );
}

#[tokio::test]
async fn a_sources_file_with_its_catalogs_under_a_key_does_not_read() {
    let t = boot().await;
    let nested = serde_json::json!({ "api": "https://api.xyz.net/f/", "feed": "https://feed.xyz.net/f/",
        "catalogs": { "ps5": { "platform": "PS5", "forums": [] } } });
    t.http.serve(SOURCES_URL, nested.to_string());
    set_sources(&t, SOURCES_URL).await.unwrap();
    let rows = rows_once(&t, |r| r.iter().any(|row| row.starts_with("Fetch failed"))).await;
    assert_eq!(rows[0], "List: the bundled list");
    assert!(
        rows[1].starts_with("Fetch failed: this is not a catalog sources file in the current format: "),
        "{rows:?}"
    );
}

#[tokio::test]
async fn a_sources_address_on_a_host_the_manifest_does_not_name_is_refused() {
    let t = boot().await;
    let refused = set_sources(&t, "https://other.example/catalog-sources.json").await.unwrap_err();
    assert!(refused.to_string().contains("other.example"), "{refused}");
    assert!(set_sources(&t, "http://xyz.dev/catalog-sources.json").await.is_err(), "https only");
}
