//! The theme's files: the fonts it ships and the animation times it uses (checked with the
//! motion helpers in `motion.rs`).

/// The Classic theme's folder, and its .slint files.
fn theme_sources() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    let theme = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/themes/classic/theme");
    let files = crate::common::slint_files(&theme);
    assert!(files.len() > 10, "the theme's files: {files:?}");
    (theme, files)
}

#[test]
fn every_font_the_theme_names_ships_with_it() {
    let (theme, files) = theme_sources();
    let shipped: Vec<String> = std::fs::read_dir(theme.join("fonts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_lowercase())
        .collect();
    let mut wrong = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (number, line) in source.lines().enumerate() {
            let Some(at) = line.find("font-family:") else { continue };
            let parts: Vec<&str> = line[at..].split('"').collect();
            // Every other part is quoted; one after == or != is a value compared, not a font.
            for pair in parts.chunks(2).filter(|p| p.len() == 2) {
                let (before, name) = (pair[0].trim_end(), pair[1]);
                if before.ends_with("==") || before.ends_with("!=") {
                    continue;
                }
                let file_name = name.replace(' ', "").to_lowercase();
                if !shipped.iter().any(|f| f.starts_with(&file_name)) {
                    wrong.push(format!("{}:{}: {name}", file.strip_prefix(&theme).unwrap().display(), number + 1));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "fonts the theme doesn't ship fall back to whatever the system has: {wrong:?}");
}

#[test]
fn every_animation_time_goes_through_look_timed_so_animations_off_cuts_it() {
    let (theme, files) = theme_sources();
    let wrong = crate::motion::times_outside_look_timed(&theme, &files);
    assert!(wrong.is_empty(), "times outside Look.timed:\n{}", wrong.join("\n"));
}
