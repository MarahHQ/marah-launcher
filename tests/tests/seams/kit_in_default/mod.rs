//! The kit's own pages (Settings, Downloads, the on-screen keyboard, Emulators and Themes) in
//! the Default theme: the Classic theme's tests of them, the same files, run on a host whose
//! `composed` starts the Default theme. Default is the theme a player sees first.
// The helpers serve the Classic theme's tests too; these suites use only some of them.
#![allow(dead_code)]

#[path = "../classic_theme/downloads.rs"]
#[allow(clippy::duplicate_mod)] // The Classic tests again, on purpose: here they run in Default.
mod downloads;
#[path = "../classic_theme/emulators.rs"]
#[allow(clippy::duplicate_mod)] // The Classic tests again, on purpose: here they run in Default.
mod emulators;
#[path = "../classic_theme/keyboard.rs"]
#[allow(clippy::duplicate_mod)] // The Classic tests again, on purpose: here they run in Default.
mod keyboard;
#[path = "../classic_theme/settings.rs"]
#[allow(clippy::duplicate_mod)] // The Classic tests again, on purpose: here they run in Default.
mod settings;

use crate::common::*;
use launcher_core::{Area, Command, Pref, SessionCmd};
use slint_interpreter::ComponentHandle;

/// The pad's way from Home to the Library in the Default theme: Cross on "All games".
const LIBRARY_BY_PAD: &[launcher_ui_host::PadButton] = &[launcher_ui_host::PadButton::South];

/// The core and the shell with no theme saved: they start the Default theme, the fallback.
fn composed(build: impl FnOnce(launcher_testkit::FakeHost) -> launcher_testkit::FakeHost) -> Composed {
    composed_first_run(build)
}

include!("../classic_theme/helpers.rs");
