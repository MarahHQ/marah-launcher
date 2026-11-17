//! Motion over the kit and every built-in theme: with Animations off, nothing moves.

use std::path::{Path, PathBuf};

use crate::common::slint_files;

/// `text` without its `Look.timed(...)` calls.
fn without_timed(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();
    while let Some(at) = rest.find("Look.timed(") {
        out.push_str(&rest[..at]);
        let mut depth = 0;
        let mut end = rest.len();
        for (i, c) in rest[at..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' if depth == 1 => {
                    end = at + i + 1;
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The times in `text` that are not 0, such as "40ms" or "6s".
fn fixed_times(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let after_name = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '-' || chars[i - 1] == '_');
        if !chars[i].is_ascii_digit() || after_name {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        let number: String = chars[start..i].iter().collect();
        let unit: String = chars[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
        if (unit == "ms" || unit == "s") && number.parse::<f64>().is_ok_and(|n| n != 0.0) {
            found.push(format!("{number}{unit}"));
        }
        i += unit.len();
    }
    found
}

/// Each `duration:` or `delay:` in `files` (under `theme`) with a time outside `Look.timed`, as
/// `file:line: value`. With Animations off the motion tokens are 0, so
/// "Tokens.motion-normal - 20ms" is -20 ms and a bare "300ms" stays 300 ms; `Look.timed` makes
/// both 0.
pub(crate) fn times_outside_look_timed(theme: &std::path::Path, files: &[std::path::PathBuf]) -> Vec<String> {
    let mut wrong = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(file).unwrap();
        for (number, line) in source.lines().enumerate() {
            for field in ["duration:", "delay:"] {
                let Some(at) = line.find(field) else { continue };
                let value = &line[at + field.len()..];
                let value = &value[..value.find([';', '}']).unwrap_or(value.len())];
                if !fixed_times(&without_timed(value)).is_empty() {
                    let name = file.strip_prefix(theme).unwrap().display();
                    wrong.push(format!("{name}:{}: {}", number + 1, value.trim()));
                }
            }
        }
    }
    wrong
}

/// The kit's folder and each built-in theme's, with their `.slint` files.
fn sources() -> Vec<(PathBuf, Vec<PathBuf>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    ["crates/ui-api/kit", "plugins/themes/default/theme", "plugins/themes/classic/theme"]
        .into_iter()
        .map(|dir| {
            let dir = root.join(dir);
            let files = slint_files(&dir);
            assert!(!files.is_empty(), "no .slint files in {}", dir.display());
            (dir, files)
        })
        .collect()
}

#[test]
fn every_animation_time_in_the_kit_goes_through_look_timed() {
    let (kit, files) = sources().swap_remove(0);
    let wrong = times_outside_look_timed(&kit, &files);
    assert!(wrong.is_empty(), "times outside Look.timed:\n{}", wrong.join("\n"));
}

/// `animation-tick()` keeps running with the motion tokens at 0: the drifting walls and the
/// turning rings read the time through `Look.clock`, which stands still with Animations off.
#[test]
fn what_moves_on_its_own_reads_look_clock_so_animations_off_stops_it() {
    let mut wrong = Vec::new();
    for (dir, files) in sources() {
        for file in files {
            if file.ends_with("kit/look.slint") {
                continue;
            }
            let source = std::fs::read_to_string(&file).unwrap();
            for (number, line) in source.lines().enumerate() {
                if line.contains("animation-tick()") {
                    wrong.push(format!("{}:{}", file.strip_prefix(&dir).unwrap().display(), number + 1));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "animation-tick() outside Look.clock:\n{}", wrong.join("\n"));
}

/// What comes in waits for its first frame through `Reveal` (`kit/reveal.slint`): a copy of its
/// 16 ms timer elsewhere is a copy that drifts from it.
#[test]
fn only_reveal_waits_for_the_first_frame() {
    let mut wrong = Vec::new();
    for (dir, files) in sources() {
        for file in files {
            if file.ends_with("kit/reveal.slint") {
                continue;
            }
            let source = std::fs::read_to_string(&file).unwrap();
            for (number, line) in source.lines().enumerate() {
                if line.contains("interval: 16ms") {
                    wrong.push(format!("{}:{}", file.strip_prefix(&dir).unwrap().display(), number + 1));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "a first-frame timer outside Reveal:\n{}", wrong.join("\n"));
}
