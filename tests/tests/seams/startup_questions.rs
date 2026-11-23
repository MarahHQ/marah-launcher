//! The start's questions, through the core: each plugin that asks is asked once a run, one
//! plugin at a time, in the plugins' order.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use launcher_core::{Area, Command, LibraryCmd};
use launcher_testkit::{FakeHost, StubPlugin, TestCore};

/// A plugin that asks a question at the start, and keeps the answers.
#[derive(Default)]
struct Asker {
    title: String,
    answers: Mutex<Vec<bool>>,
}

#[launcher_api::async_trait]
impl launcher_api::StartupWork for Asker {
    async fn status(&self) -> Option<launcher_api::StartupStep> {
        None
    }

    async fn question(&self) -> Option<launcher_api::StartupQuestion> {
        self.answers.lock().unwrap().is_empty().then(|| launcher_api::StartupQuestion {
            title: self.title.clone(),
            text: "It downloads something.".into(),
            yes: "Install".into(),
            no: "Not now".into(),
        })
    }

    async fn answer(&self, yes: bool) {
        self.answers.lock().unwrap().push(yes);
    }
}

/// The asker `id`, as a plugin with the startup capability.
fn asker(id: &str) -> (StubPlugin, Arc<Asker>) {
    let work = Arc::new(Asker { title: format!("Install {id}?"), answers: Mutex::default() });
    let provided = work.clone();
    let plugin = StubPlugin::from_toml(&format!(
        "id = \"{id}\"\nkind = \"service\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.6\"\n\
         capabilities = [\"startup\"]"
    ))
    .provides(move |_| Ok(launcher_api::Provided { startup: Some(provided.clone()), ..Default::default() }));
    (plugin, work)
}

/// Two askers, booted: the one the start asks first, then the other.
async fn two_askers() -> (TestCore, Arc<Asker>, Arc<Asker>) {
    let ((a, one), (b, other)) = (asker("asker-a"), asker("asker-b"));
    let test = FakeHost::new().plugin(a).plugin(b).boot_core().await;
    test.core.dispatch(Command::Library(LibraryCmd::Refresh)).await.unwrap();
    test.until(Area::Library, |core| core.view().question.is_some()).await;
    if question(&test).as_deref() == Some(other.title.as_str()) {
        return (test, other, one);
    }
    (test, one, other)
}

fn question(test: &TestCore) -> Option<String> {
    test.core.view().question.map(|q| q.title)
}

async fn asked(test: &TestCore, title: &str) {
    test.until(Area::Library, |core| core.view().question.is_some_and(|q| q.title == title)).await;
}

#[tokio::test]
async fn two_plugins_that_ask_are_both_asked_one_after_the_other() {
    let (test, first, second) = two_askers().await;

    asked(&test, &first.title).await;
    test.core.dispatch(Command::AnswerQuestion(true)).await.unwrap();
    asked(&test, &second.title).await;
    assert!(!test.core.view().boot.ready, "the boot screen waits for the second answer");
    test.core.dispatch(Command::AnswerQuestion(false)).await.unwrap();
    test.until(Area::Library, |core| core.view().boot.ready).await;

    assert_eq!(*first.answers.lock().unwrap(), [true]);
    assert_eq!(*second.answers.lock().unwrap(), [false]);
    assert_eq!(question(&test), None, "each is asked once a run");
}

/// A first answer that comes after the boot screen stopped waiting still brings the second
/// question.
#[tokio::test]
async fn a_late_first_answer_still_brings_the_second_question() {
    let (test, first, second) = two_askers().await;
    asked(&test, &first.title).await;

    test.host.clock.advance(Duration::from_secs(10 * 60)).await;
    test.until(Area::Library, |core| core.view().boot.ready).await;
    test.core.dispatch(Command::AnswerQuestion(false)).await.unwrap();

    asked(&test, &second.title).await;
    test.core.dispatch(Command::AnswerQuestion(true)).await.unwrap();
    assert_eq!(*first.answers.lock().unwrap(), [false]);
    assert_eq!(*second.answers.lock().unwrap(), [true]);
    assert_eq!(question(&test), None);
}
