//! Seam 3: the Classic theme, driven through its accessible labels and keys, one module per
//! screen. The setup and the steps more than one screen uses are here.

mod catalog;
mod classic_only;
mod downloads;
mod emulators;
mod game_hub;
mod home;
mod installs;
mod keyboard;
mod library;
mod notices;
mod play;
mod releases;
mod settings;
pub(crate) mod theme_files;
mod trailer;

use crate::common::*;
use launcher_core::{Area, Command, Pref, SessionCmd};
use slint_interpreter::ComponentHandle;

/// The pad's way from Home to the Library in the Classic theme: up to the top bar, right to
/// Library, Cross.
const LIBRARY_BY_PAD: &[launcher_ui_host::PadButton] =
    &[launcher_ui_host::PadButton::Up, launcher_ui_host::PadButton::Right, launcher_ui_host::PadButton::South];

include!("helpers.rs");
