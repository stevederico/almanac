//! Atom 1.0 feeds, for notes. Calendar apps have no standard place for a note,
//! but every feed reader can subscribe to this.

/// XML-safe text. Characters XML 1.0 forbids (most control characters,
/// U+FFFE, U+FFFF) are dropped, then the five specials are escaped.
pub fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

pub struct AtomEntry {
    pub id: String,
    pub title: String,
    /// RFC 3339.
    pub updated: String,
    pub published: String,
    pub content: String,
    pub tags: Vec<String>,
}

/// `updated` is the feed's own timestamp: the newest entry's, or now.
pub fn render_atom(feed_id: &str, title: &str, updated: &str, entries: &[AtomEntry]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<feed xmlns=\"http://www.w3.org/2005/Atom\">\n");
    out.push_str(&format!("  <id>{}</id>\n", xml_escape(feed_id)));
    out.push_str(&format!("  <title>{}</title>\n", xml_escape(title)));
    out.push_str(&format!("  <updated>{}</updated>\n", xml_escape(updated)));
    out.push_str("  <author><name>Almanac</name></author>\n");
    for e in entries {
        out.push_str("  <entry>\n");
        out.push_str(&format!("    <id>{}</id>\n", xml_escape(&e.id)));
        out.push_str(&format!("    <title>{}</title>\n", xml_escape(&e.title)));
        out.push_str(&format!("    <updated>{}</updated>\n", xml_escape(&e.updated)));
        out.push_str(&format!("    <published>{}</published>\n", xml_escape(&e.published)));
        for tag in &e.tags {
            out.push_str(&format!("    <category term=\"{}\"/>\n", xml_escape(tag)));
        }
        out.push_str(&format!(
            "    <content type=\"text\">{}</content>\n",
            xml_escape(&e.content)
        ));
        out.push_str("  </entry>\n");
    }
    out.push_str("</feed>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_five_specials() {
        assert_eq!(xml_escape(r#"a & b < c > d " e ' f"#), "a &amp; b &lt; c &gt; d &quot; e &apos; f");
    }

    #[test]
    fn drops_characters_xml_forbids_but_keeps_whitespace_and_unicode() {
        assert_eq!(xml_escape("a\u{0}b\u{8}c\u{b}d\u{1b}e\u{fffe}f"), "abcdef");
        assert_eq!(xml_escape("tab\there\nline\r\nend"), "tab\there\nline\r\nend");
        assert_eq!(xml_escape("é ✓ 日本 🎉"), "é ✓ 日本 🎉");
    }

    #[test]
    fn renders_a_feed() {
        let xml = render_atom(
            "urn:almanac:cal_1:notes",
            "Trip Notes",
            "2026-09-02T11:30:00.000Z",
            &[AtomEntry {
                id: "urn:almanac:cal_1:n1".into(),
                title: "Packing <list>".into(),
                updated: "2026-09-02T11:30:00.000Z".into(),
                published: "2026-09-01T10:00:00.000Z".into(),
                content: "socks & shoes\n- passport".into(),
                tags: vec!["travel".into()],
            }],
        );
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <feed xmlns=\"http://www.w3.org/2005/Atom\">\n\
             \x20 <id>urn:almanac:cal_1:notes</id>\n\
             \x20 <title>Trip Notes</title>\n\
             \x20 <updated>2026-09-02T11:30:00.000Z</updated>\n\
             \x20 <author><name>Almanac</name></author>\n\
             \x20 <entry>\n\
             \x20   <id>urn:almanac:cal_1:n1</id>\n\
             \x20   <title>Packing &lt;list&gt;</title>\n\
             \x20   <updated>2026-09-02T11:30:00.000Z</updated>\n\
             \x20   <published>2026-09-01T10:00:00.000Z</published>\n\
             \x20   <category term=\"travel\"/>\n\
             \x20   <content type=\"text\">socks &amp; shoes\n- passport</content>\n\
             \x20 </entry>\n\
             </feed>\n"
        );
    }

    #[test]
    fn a_hostile_title_stays_text() {
        let xml = render_atom(
            "id",
            "</title><evil/>",
            "2026-09-02T11:30:00.000Z",
            &[AtomEntry {
                id: "x".into(),
                title: "]]></title><script>".into(),
                updated: "u".into(),
                published: "p".into(),
                content: "</content><evil/>".into(),
                tags: vec!["a\"/><evil x=\"".into()],
            }],
        );
        assert!(!xml.contains("<evil"), "{xml}");
        assert!(!xml.contains("<script"), "{xml}");
    }
}
