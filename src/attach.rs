//! Ticket attachments: pull image references out of an issue body and fetch the
//! bytes. GitHub rewrites pasted images to `github.com/user-attachments/assets/<id>`
//! (and older `user-images.githubusercontent.com`) URLs; the former 302-redirects
//! to a signed S3 URL only when the request carries the `gh` token, so fetching is
//! a two-step dance (see `fetch_image`).

use std::io::Read;
use std::time::Duration;

/// One image referenced in a body: its URL plus any author-declared pixel size
/// (from an `<img width= height=>` tag), which lets the detail pane reserve the
/// right height before the bytes have downloaded.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A run of the body in reading order: either a block of text or an image.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentPart {
    Text(String),
    Image(ImageRef),
}

/// Split a body into text/image parts in order. Recognises Markdown `![alt](url)`
/// and HTML `<img … src="url" …>`; everything else stays text.
pub fn split_content(body: &str) -> Vec<ContentPart> {
    let mut parts = Vec::new();
    let mut rest = body;

    while let Some(found) = next_image(rest) {
        let (start, end, image) = found;
        if start > 0 {
            parts.push(ContentPart::Text(rest[..start].to_string()));
        }
        parts.push(ContentPart::Image(image));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        parts.push(ContentPart::Text(rest.to_string()));
    }
    parts
}

/// Every image URL in a body, in order — the detail loader uses this to know what
/// to fetch without caring about the surrounding text.
pub fn image_urls(body: &str) -> Vec<String> {
    split_content(body)
        .into_iter()
        .filter_map(|p| match p {
            ContentPart::Image(i) => Some(i.url),
            ContentPart::Text(_) => None,
        })
        .collect()
}

/// Find the earliest image token in `s`, returning `(start, end, image)` byte
/// offsets so the caller can slice the text around it.
fn next_image(s: &str) -> Option<(usize, usize, ImageRef)> {
    let md = find_markdown(s);
    let html = find_html(s);
    match (md, html) {
        (Some(m), Some(h)) => Some(if m.0 <= h.0 { m } else { h }),
        (Some(m), None) => Some(m),
        (None, Some(h)) => Some(h),
        (None, None) => None,
    }
}

/// `![alt](url)` — the `!` distinguishes an image from a plain `[text](link)`.
fn find_markdown(s: &str) -> Option<(usize, usize, ImageRef)> {
    let mut from = 0;
    loop {
        let bang = s[from..].find("![")? + from;
        let after_alt = s[bang..].find("](").map(|i| bang + i + 2);
        let Some(url_start) = after_alt else {
            from = bang + 2;
            continue;
        };
        let rel_close = s[url_start..].find(')')?;
        let url_end = url_start + rel_close;
        // A newline before the closing paren means this wasn't a real link.
        let url = s[url_start..url_end].trim();
        if url.contains('\n') || url.is_empty() {
            from = bang + 2;
            continue;
        }
        return Some((
            bang,
            url_end + 1,
            ImageRef {
                url: url.to_string(),
                width: None,
                height: None,
            },
        ));
    }
}

/// `<img … src="url" width="632" height="637" …>` (attribute order-independent,
/// single or double quotes, case-insensitive tag).
fn find_html(s: &str) -> Option<(usize, usize, ImageRef)> {
    let lower = s.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let tag_start = lower[from..].find("<img").map(|i| from + i)?;
        let rel_gt = lower[tag_start..].find('>')?;
        let tag_end = tag_start + rel_gt + 1;
        let tag = &s[tag_start..tag_end];
        if let Some(url) = attr(tag, "src") {
            return Some((
                tag_start,
                tag_end,
                ImageRef {
                    url,
                    width: attr(tag, "width").and_then(|v| v.parse().ok()),
                    height: attr(tag, "height").and_then(|v| v.parse().ok()),
                },
            ));
        }
        from = tag_end;
    }
}

/// Read `name="value"` (or single-quoted) out of an HTML tag, case-insensitively.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let mut search = 0;
    loop {
        let at = lower[search..].find(&key)? + search;
        // Attribute names are whitespace-separated (and may contain `-`), so only a
        // leading space counts — this rejects `data-src=` when asked for `src=`.
        let boundary_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let vstart = at + key.len();
        if !boundary_ok {
            search = vstart;
            continue;
        }
        let bytes = tag.as_bytes();
        let quote = *bytes.get(vstart)?;
        if quote != b'"' && quote != b'\'' {
            search = vstart;
            continue;
        }
        let rest = &tag[vstart + 1..];
        let close = rest.find(quote as char)?;
        return Some(rest[..close].to_string());
    }
}

/// Fetch the raw bytes of an attachment. `token` is the `gh` OAuth token, needed
/// for `user-attachments` URLs (they 302 to a signed S3 URL only when authorised).
/// Done in two steps so the token is never re-sent to S3, which rejects requests
/// carrying two auth mechanisms.
pub fn fetch_image(url: &str, token: Option<&str>) -> anyhow::Result<Vec<u8>> {
    const MAX_BYTES: u64 = 25 * 1024 * 1024;

    let no_follow = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(20))
        .build();
    let mut req = no_follow.get(url).set("User-Agent", "lazytickets");
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let resp = req.call()?;

    let bytes = match resp.status() {
        200 => resp,
        300..=399 => {
            let location = resp
                .header("Location")
                .ok_or_else(|| anyhow::anyhow!("redirect without a Location header"))?
                .to_string();
            ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(20))
                .build()
                .get(&location)
                .set("User-Agent", "lazytickets")
                .call()?
        }
        other => anyhow::bail!("unexpected HTTP status {other}"),
    };

    let mut buf = Vec::new();
    bytes.into_reader().take(MAX_BYTES).read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_html_img_with_dimensions() {
        let body = "before\n<img width=\"632\" height=\"637\" alt=\"Image\" src=\"https://github.com/user-attachments/assets/abc\" />\nafter";
        let parts = split_content(body);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], ContentPart::Text("before\n".into()));
        assert_eq!(
            parts[1],
            ContentPart::Image(ImageRef {
                url: "https://github.com/user-attachments/assets/abc".into(),
                width: Some(632),
                height: Some(637),
            })
        );
        assert_eq!(parts[2], ContentPart::Text("\nafter".into()));
    }

    #[test]
    fn parses_markdown_image_but_not_links() {
        let body = "see ![a shot](https://x/y.png) and [a link](https://x/z) end";
        let urls = image_urls(body);
        assert_eq!(urls, vec!["https://x/y.png".to_string()]);
    }

    #[test]
    fn handles_multiple_images_in_order() {
        let body =
            "<img src='https://a/1.png'>mid<img src='https://a/2.png'>![m](https://a/3.png)";
        assert_eq!(
            image_urls(body),
            vec![
                "https://a/1.png".to_string(),
                "https://a/2.png".to_string(),
                "https://a/3.png".to_string(),
            ]
        );
    }

    #[test]
    fn body_without_images_is_one_text_part() {
        let parts = split_content("just some text");
        assert_eq!(parts, vec![ContentPart::Text("just some text".into())]);
    }

    #[test]
    fn ignores_data_src_lookalike_attribute() {
        // `data-src` must not be picked up when we ask for `src`.
        let tag = "<img data-src=\"decoy\" src=\"real.png\">";
        assert_eq!(attr(tag, "src").as_deref(), Some("real.png"));
    }
}
