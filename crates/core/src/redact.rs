//! Takes personal data out of text that leaves this computer, such as the end of an emulator's
//! log in a report to a community list: the home folder becomes `~`, and the user name, the
//! computer's name, IP addresses and emails become `<user>`, `<computer>`, `<ip>` and
//! `<email>`. It takes out too much rather than too little.

use std::path::Path;

/// What is personal on this computer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Personal {
    /// The home folder, such as `/home/alice`.
    pub home: Option<String>,
    /// The user name: the home folder's last part.
    pub user: Option<String>,
    /// The computer's name on the network.
    pub computer: Option<String>,
}

impl Personal {
    /// The personal data of a computer whose home folder is `home` and whose name is `computer`
    /// (empty when the system doesn't say).
    pub fn new(home: Option<&Path>, computer: &str) -> Self {
        let home = home.map(|h| h.display().to_string()).filter(|h| h.len() > 1);
        let user = home.as_deref().and_then(|h| h.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next());
        let user = user.filter(|u| !u.is_empty()).map(str::to_owned);
        let computer = Some(computer.trim().to_owned()).filter(|c| !c.is_empty());
        Self { home, user, computer }
    }
}

/// `text` without the personal data of `who`, nor IP addresses or emails.
pub(crate) fn redact(text: &str, who: &Personal) -> String {
    let mut out = text.to_owned();
    // The home folder first, so the user name in it becomes `~` too. The same folder under
    // another system's home, such as Fedora Atomic's /var/home, goes too.
    let mut homes: Vec<String> = who.home.iter().cloned().collect();
    if let Some(user) = &who.user {
        for root in ["/var/home/", "/home/", "/Users/", "C:\\Users\\", "C:/Users/"] {
            homes.push(format!("{root}{user}"));
        }
    }
    // The longest first: /home/alice is inside /var/home/alice.
    homes.sort_by_key(|h| std::cmp::Reverse(h.len()));
    for home in homes {
        out = replace_path(&out, &home);
    }
    out = words(&out, emails);
    out = words(&out, ip_addresses);
    // The computer first: its name often holds the user's, as alice-desktop does.
    for (name, mark) in [(&who.computer, "<computer>"), (&who.user, "<user>")] {
        if let Some(name) = name {
            out = replace_word(&out, name, mark);
        }
    }
    out
}

/// `text` with `home` as `~`, where a path starts with it.
fn replace_path(text: &str, home: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(home) {
        let after = &rest[at + home.len()..];
        let ends = after.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.');
        out.push_str(&rest[..at]);
        out.push_str(if ends { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `text` with `name` as `mark` wherever it is a whole word, in any case.
fn replace_word(text: &str, name: &str, mark: &str) -> String {
    let lower = text.to_lowercase();
    let wanted = name.to_lowercase();
    // Lowercase can change a text's length; then the places in it don't match the text's.
    if wanted.is_empty() || lower.len() != text.len() {
        return text.to_owned();
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut search = 0;
    while let Some(found) = lower[search..].find(&wanted) {
        let at = search + found;
        let end = at + wanted.len();
        if !word(text[..at].chars().next_back()) && !word(text[end..].chars().next()) {
            out.push_str(&text[from..at]);
            out.push_str(mark);
            from = end;
        }
        search = end;
    }
    out.push_str(&text[from..]);
    out
}

/// Runs `mark` over each word of `text`: each run of characters that aren't spaces, quotes
/// or brackets. `mark` gives a word's new text, or `None` to keep it.
fn words(text: &str, mark: fn(&str) -> Option<String>) -> String {
    let split = |c: char| c.is_whitespace() || "\"'`<>()[]{},;|=".contains(c);
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if split(c) {
            push_word(&mut out, &mut word, mark);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    push_word(&mut out, &mut word, mark);
    out
}

fn push_word(out: &mut String, word: &mut String, mark: fn(&str) -> Option<String>) {
    out.push_str(&mark(word).unwrap_or_else(|| word.clone()));
    word.clear();
}

/// An email in a word, such as `mailto:alice@example.com.`, as `<email>`.
fn emails(word: &str) -> Option<String> {
    let at = word.find('@')?;
    let local = |c: char| c.is_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_alphanumeric() || ".-".contains(c);
    let start = word[..at].rfind(|c| !local(c)).map_or(0, |i| i + 1);
    let end = word[at + 1..].find(|c| !domain(c)).map_or(word.len(), |i| at + 1 + i);
    let host = word[at + 1..end].trim_end_matches('.');
    let end = at + 1 + host.len();
    (start < at && host.contains('.')).then(|| format!("{}<email>{}", &word[..start], &word[end..]))
}

/// IP addresses in a word, such as `192.168.1.20:8080` or `[fe80::1]`, as `<ip>`.
fn ip_addresses(word: &str) -> Option<String> {
    let mut out = String::with_capacity(word.len());
    let mut found = false;
    let mut rest = word;
    while !rest.is_empty() {
        let len = ipv4_at(rest).or_else(|| ipv6_at(rest));
        let before = out.chars().next_back();
        match len {
            Some(len) if before.is_none_or(|c| !c.is_ascii_hexdigit() && c != '.' && c != ':') => {
                out.push_str("<ip>");
                rest = &rest[len..];
                found = true;
            }
            _ => {
                let c = rest.chars().next().expect("not empty");
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    found.then_some(out)
}

/// The length of the IPv4 address `text` starts with: four numbers to 255, with dots.
fn ipv4_at(text: &str) -> Option<usize> {
    let mut len = 0;
    for part in 0..4 {
        if part > 0 {
            text[len..].starts_with('.').then_some(())?;
            len += 1;
        }
        let digits = text[len..].bytes().take_while(u8::is_ascii_digit).count();
        (1..=3).contains(&digits).then_some(())?;
        text[len..len + digits].parse::<u8>().ok()?;
        len += digits;
    }
    // Not a longer run of numbers, such as a version 1.2.3.4.5.
    let rest = &text[len..];
    let more = rest.starts_with(|c: char| c.is_ascii_digit())
        || rest.starts_with('.') && rest[1..].starts_with(|c: char| c.is_ascii_digit());
    (!more).then_some(len)
}

/// The length of the IPv6 address `text` starts with: hex groups with colons, with `::` or all
/// eight groups. A time such as 12:30:05 has neither.
fn ipv6_at(text: &str) -> Option<usize> {
    let len = text.bytes().take_while(|b| b.is_ascii_hexdigit() || *b == b':').count();
    let candidate = text[..len].trim_end_matches(':');
    let groups: Vec<&str> = candidate.split(':').collect();
    let short = candidate.contains("::") && groups.iter().any(|g| !g.is_empty());
    let full = groups.len() == 8 && groups.iter().all(|g| !g.is_empty());
    let fits = groups.iter().all(|g| g.len() <= 4) && groups.len() <= 8;
    let next = text[candidate.len()..].chars().next();
    let ends = next.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    // `::1` alone could be a C++ name's end; one hex digit is too little to tell.
    (fits && ends && (short && candidate.len() >= 4 || full)).then_some(candidate.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Personal {
        Personal::new(Some(Path::new("/home/alice")), "alice-desktop")
    }

    #[test]
    fn the_home_folder_is_a_tilde() {
        let who = alice();
        assert_eq!(redact("open /home/alice/Games/a.pkg", &who), "open ~/Games/a.pkg");
        assert_eq!(redact("at /var/home/alice/x", &who), "at ~/x");
        assert_eq!(redact("/home/alice", &who), "~");
        assert_eq!(redact("/home/alicex/a", &who), "/home/alicex/a", "another user's home stays");
    }

    #[test]
    fn the_user_and_computer_names_go_as_whole_words() {
        let who = alice();
        assert_eq!(redact("user Alice logged in on alice-desktop", &who), "user <user> logged in on <computer>");
        assert_eq!(redact("malice", &who), "malice", "a part of a word stays");
    }

    #[test]
    fn emails_and_ip_addresses_go() {
        let who = Personal::default();
        assert_eq!(redact("mail bob.smith+x@mail.example.com.", &who), "mail <email>.");
        assert_eq!(redact("peer 192.168.1.20:8080 up", &who), "peer <ip>:8080 up");
        assert_eq!(redact("[fe80::1ff:fe23:4567:890a] and 2001:db8::1", &who), "[<ip>] and <ip>");
        assert_eq!(redact("2001:0db8:85a3:0000:0000:8a2e:0370:7334", &who), "<ip>");
    }

    #[test]
    fn times_versions_and_names_stay() {
        let who = Personal::default();
        let text = "[12:30:05] shadPS4 v0.21.0 std::vector Vulkan 1.3.280 at 0x1200000200 build 1.2.3.4.5";
        assert_eq!(redact(text, &who), text);
    }

    #[test]
    fn with_no_home_nothing_of_the_user_goes() {
        let who = Personal::new(None, "");
        assert_eq!(redact("/home/alice/a", &who), "/home/alice/a");
    }
}
