//! Text fits its box: a `Text` with a fixed font size inside an element of a fixed height has
//! room for its line. At the largest "Size of everything" the text and the boxes scale alike
//! (`Look.t` is `Look.k` once that is past 0.75), so a line too tall here is cut there too.

use std::path::Path;

use crate::common::slint_files;

/// A line of text is about 1.2 times its font size (Inter 1.21).
const LINE: f64 = 1.2;

/// One element: its type, the line it starts on, its fixed height and font size in canvas
/// pixels when it has them, and its children.
#[derive(Debug, Default)]
struct Element {
    kind: String,
    line: usize,
    height: Option<f64>,
    font_size: Option<f64>,
    children: Vec<Element>,
}

/// `N * Look.px` or `N * Look.tpx`: N.
fn canvas_pixels(value: &str) -> Option<f64> {
    let value = value.trim().trim_end_matches(';').trim();
    let (number, unit) = value.split_once(" * ")?;
    matches!(unit, "Look.px" | "Look.tpx").then(|| number.parse().ok()).flatten()
}

/// The elements of `source`, as a tree under a root.
fn parse(source: &str) -> Element {
    let mut stack = vec![Element { kind: "root".into(), ..Element::default() }];
    for (number, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("").trim();
        let opens = line.ends_with('{');
        let closes = line.starts_with('}');
        if closes && stack.len() > 1 {
            let done = stack.pop().unwrap();
            stack.last_mut().unwrap().children.push(done);
        }
        if opens && !closes {
            // `name := Type {`, `if cond : Type {`, `for x in y : Type {`, `Type {`.
            let head = line.trim_end_matches('{').trim();
            let head = head.rsplit(" : ").next().unwrap_or(head);
            let head = head.rsplit(":= ").next().unwrap_or(head).trim();
            let kind = head.split_whitespace().last().unwrap_or("").to_owned();
            stack.push(Element { kind, line: number + 1, ..Element::default() });
            continue;
        }
        let Some(element) = stack.last_mut() else { continue };
        if let Some(value) = line.strip_prefix("height:") {
            element.height = canvas_pixels(value);
        } else if let Some(value) = line.strip_prefix("font-size:") {
            element.font_size = canvas_pixels(value);
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(done);
    }
    stack.pop().unwrap()
}

/// Each `Text` too tall for the fixed height of the element it is in, as `line: what`. Returns
/// how many it checked.
fn too_tall(element: &Element, out: &mut Vec<String>) -> usize {
    let mut checked = 0;
    for child in &element.children {
        if let (Some(height), "Text", Some(size), None) =
            (element.height, child.kind.as_str(), child.font_size, child.height)
        {
            checked += 1;
            if size * LINE > height + 0.5 {
                out.push(format!("{}: a {size}px line in a {height}px {}", child.line, element.kind));
            }
        }
        if let ("Text", Some(height), Some(size)) = (child.kind.as_str(), child.height, child.font_size) {
            checked += 1;
            if size * LINE > height + 0.5 {
                out.push(format!("{}: a {size}px line in its own {height}px", child.line));
            }
        }
        checked += too_tall(child, out);
    }
    checked
}

#[test]
fn every_line_of_text_fits_the_box_it_is_in() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let (mut wrong, mut checked) = (Vec::new(), 0);
    for dir in ["crates/ui-api/kit", "plugins/themes/default/theme", "plugins/themes/classic/theme"] {
        for file in slint_files(&root.join(dir)) {
            let mut found = Vec::new();
            checked += too_tall(&parse(&std::fs::read_to_string(&file).unwrap()), &mut found);
            let name = file.strip_prefix(&root).unwrap().display().to_string();
            wrong.extend(found.into_iter().map(|f| format!("{name}:{f}")));
        }
    }
    assert!(wrong.is_empty(), "text taller than its box:\n{}", wrong.join("\n"));
    println!("{checked} lines of text in a box of a fixed height");
    assert!(checked > 10, "the parser found only {checked} lines of text in a box: is it still reading the files?");
}

#[test]
fn the_parser_finds_a_line_too_tall_and_lets_one_that_fits_pass() {
    let source = "component A inherits Rectangle {\n    Rectangle {\n        height: 20 * Look.px;\n        Text {\n            font-size: 18 * Look.tpx;\n        }\n    }\n    Rectangle {\n        height: 30 * Look.px;\n        Text {\n            font-size: 18 * Look.px;\n        }\n    }\n}\n";
    let mut found = Vec::new();
    too_tall(&parse(source), &mut found);
    assert_eq!(found, ["4: a 18px line in a 20px Rectangle"]);
}
