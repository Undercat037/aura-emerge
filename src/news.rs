//! Arch Linux news (eselect-news style). RSS via http helper, hand-rolled
//! tag parse. Read state in ~/.cache/aura-emerge/news.state (no root).
//!

use crate::theme::Themed;
use colored::Colorize;
use std::fs;
use std::io::Write;

const NEWS_FEED_URL: &str = "https://archlinux.org/feeds/news/";
/// Cap for list view.
const LIST_LIMIT: usize = 30;

pub(crate) struct NewsItem {
    pub title: String,
    pub link: String,
    pub pub_date: String,
    pub description: String,
    pub guid: String,
}

fn fetch_feed() -> Option<String> {
    let text = crate::http::get(NEWS_FEED_URL, 10)?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn split_items(xml: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<item>") {
        let after_start = &rest[start + "<item>".len()..];
        if let Some(end) = after_start.find("</item>") {
            items.push(&after_start[..end]);
            rest = &after_start[end + "</item>".len()..];
        } else {
            break;
        }
    }
    items
}

/// Text of `<tag ...>content</tag>` (tolerates attributes on open tag).
fn extract_tag(block: &str, tag: &str) -> Option<String> {
    let open_needle = format!("<{}", tag);
    let mut search_from = 0;
    loop {
        let rel = block[search_from..].find(&open_needle)?;
        let start = search_from + rel;
        let after = &block[start + open_needle.len()..];
        let next_char = after.chars().next()?;
        // Avoid matching "<title" inside "<titleFoo".
        if next_char != '>' && next_char != '/' && !next_char.is_whitespace() {
            search_from = start + open_needle.len();
            continue;
        }
        let gt = after.find('>')?;
        let content_start = &after[gt + 1..];
        let close_needle = format!("</{}>", tag);
        let end = content_start.find(&close_needle)?;
        return Some(content_start[..end].trim().to_string());
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&") // last, avoid double-decode
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drop C0/C1 controls (and TAB/LF so fields stay one line), DEL, and
/// common bidi/isolate overrides that can reverse or hide terminal
/// output. Newlines in a single field would also forge multi-line

fn sanitize_text(s: &str) -> String {
    // Turn line breaks into spaces *before* dropping other controls so a
    // forged "guid\nextra-guid" cannot glue into one opaque token.
    let spaced: String = s
        .chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect();
    spaced
        .chars()
        .filter(|c| match *c {
            c if c.is_control() => false,
            // U+202A..U+202E (embedding/override), U+2066..U+2069 (isolates)
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => false,
            // BOM / zero-width joiners
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' => false,
            _ => true,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_news(xml: &str) -> Vec<NewsItem> {
    split_items(xml)
        .into_iter()
        .filter_map(|block| {
            let title = sanitize_text(&decode_entities(&extract_tag(block, "title")?));
            let link = sanitize_text(&extract_tag(block, "link")?);
            let pub_date = sanitize_text(&extract_tag(block, "pubDate").unwrap_or_default());
            let description = extract_tag(block, "description")
                .map(|d| sanitize_text(&strip_tags(&decode_entities(&d))))
                .unwrap_or_default();
            let guid = sanitize_text(&extract_tag(block, "guid").unwrap_or_else(|| link.clone()));
            if title.is_empty() {
                return None;
            }
            Some(NewsItem {
                title,
                link,
                pub_date,
                description,
                guid,
            })
        })
        .collect()
}

/// RFC 822 date → "21 Jul 2026"; else raw string.
fn short_date(pub_date: &str) -> String {
    let parts: Vec<&str> = pub_date.split_whitespace().collect();
    if parts.len() >= 4 {
        format!("{} {} {}", parts[1], parts[2], parts[3])
    } else {
        pub_date.to_string()
    }
}

fn state_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::Path::new(&home).join(".cache/aura-emerge/news.state"))
}

fn load_read_guids() -> std::collections::HashSet<String> {
    let Some(path) = state_path() else {
        return Default::default();
    };
    match fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .map(|l| sanitize_text(l.trim()))
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Default::default(),
    }
}

fn save_read_guids(guids: &std::collections::HashSet<String>) {
    let Some(path) = state_path() else {
        eprintln!(">>> Warning: could not determine $HOME, news read-state not saved");
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            eprintln!(
                ">>> Warning: could not create {}, news read-state not saved",
                parent.display()
            );
            return;
        }
    }
    let tmp = path.with_extension("tmp");
    let write_result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        for g in guids {
            writeln!(f, "{}", sanitize_text(g))?;
        }
        Ok(())
    })();
    if write_result.is_err() || fs::rename(&tmp, &path).is_err() {
        eprintln!(">>> Warning: failed to save news read-state");
    }
}

/// Unread count for a pre-`-u` heads-up. `None` on failure (never blocks upgrade).
pub(crate) fn unread_count_quiet() -> Option<usize> {
    let xml = fetch_feed()?;
    let mut items = parse_news(&xml);
    if items.len() > LIST_LIMIT {
        items.truncate(LIST_LIMIT);
    }
    let read = load_read_guids();
    Some(items.iter().filter(|it| !read.contains(&it.guid)).count())
}

/// `--news` arg: "" = list, "all" = mark all read, N = show item N.
pub(crate) fn run_news(arg: &str) {
    println!("{} Fetching Arch Linux news...", ">>>".t_green().bold());
    let Some(xml) = fetch_feed() else {
        eprintln!(">>> Error: could not fetch {}", NEWS_FEED_URL);
        std::process::exit(1);
    };
    let mut items = parse_news(&xml);
    if items.len() > LIST_LIMIT {
        items.truncate(LIST_LIMIT);
    }
    if items.is_empty() {
        println!(">>> No news items found.");
        return;
    }

    let mut read = load_read_guids();

    match arg.trim() {
        "" => list_news(&items, &read),
        "all" => {
            for it in &items {
                read.insert(it.guid.clone());
            }
            save_read_guids(&read);
            println!(
                "{} Marked {} item(s) as read.",
                ">>>".t_green().bold(),
                items.len()
            );
        }
        n => match n.parse::<usize>() {
            Ok(idx) if idx >= 1 && idx <= items.len() => {
                let item = &items[idx - 1];
                print_full(item);
                read.insert(item.guid.clone());
                save_read_guids(&read);
            }
            _ => {
                eprintln!(
                    ">>> Error: '{}' is not a valid item number (1-{})",
                    n,
                    items.len()
                );
                std::process::exit(1);
            }
        },
    }
}

fn list_news(items: &[NewsItem], read: &std::collections::HashSet<String>) {
    println!(
        "{} Arch Linux News (https://archlinux.org/news/)",
        ">>>".t_green().bold()
    );
    let mut unread_count = 0;
    for (i, it) in items.iter().enumerate() {
        let is_unread = !read.contains(&it.guid);
        if is_unread {
            unread_count += 1;
        }
        let marker = if is_unread {
            "N".t_yellow().bold().to_string()
        } else {
            " ".to_string()
        };
        println!(
            "  {:>2}  [{}]  {}  {}",
            i + 1,
            marker,
            short_date(&it.pub_date).dimmed(),
            if is_unread {
                it.title.bold().to_string()
            } else {
                it.title.clone()
            }
        );
    }
    println!();
    if unread_count > 0 {
        println!(
            "{} {} new news item(s). Use `emerge --news <N>` to read one, `emerge --news all` to dismiss all.",
            ">>>".t_green().bold(),
            unread_count
        );
    } else {
        println!("{} No new news items.", ">>>".t_green().bold());
    }
}

fn print_full(item: &NewsItem) {
    println!("{} {}", ">>>".t_green().bold(), item.title.bold());
    println!("    {}: {}", "Date".dimmed(), short_date(&item.pub_date));
    println!("    {}: {}", "Link".dimmed(), item.link);
    println!();
    println!("{}", item.description);
}

#[cfg(test)]
mod sanitize_tests {
    use super::*;

    #[test]
    fn sanitize_strips_ansi_and_bidi() {
        let dirty = "hello\x1b[2J\x1b]0;pwned\x07 world\u{202E}reversed";
        let clean = sanitize_text(dirty);
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\u{202E}'));
        assert!(clean.contains("hello"));
        assert!(clean.contains("world"));
    }

    #[test]
    fn sanitize_collapses_newlines_so_guid_stays_one_line() {
        let forged = "legit-guid\nextra-forged-guid";
        assert_eq!(sanitize_text(forged), "legit-guid extra-forged-guid");
    }
}
