//! Canvas paginates with RFC 8288 `Link` headers: `<https://…?page=2>; rel="next", <…>; rel="last"`.
use reqwest::header::{HeaderMap, LINK};

/// The `rel="next"` URL from a response's `Link` header(s), if any.
pub(crate) fn next_link(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(LINK)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .find_map(parse_next)
}

fn parse_next(entry: &str) -> Option<String> {
    let (target, params) = entry.trim().split_once(';')?;
    let url = target.trim().strip_prefix('<')?.strip_suffix('>')?;
    let is_next = params.split(';').any(|param| {
        param.split_once('=').is_some_and(|(key, value)| {
            key.trim().eq_ignore_ascii_case("rel")
                && value
                    .trim()
                    .trim_matches('"')
                    .split_whitespace()
                    .any(|rel| rel.eq_ignore_ascii_case("next"))
        })
    });
    is_next.then(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for v in values {
            map.append(LINK, HeaderValue::from_str(v).unwrap());
        }
        map
    }

    #[test]
    fn finds_next_among_other_rels() {
        let h = headers(&[
            r#"<https://x/api/v1/courses?page=1>; rel="current", <https://x/api/v1/courses?page=2>; rel="next", <https://x/api/v1/courses?page=1>; rel="first""#,
        ]);
        assert_eq!(
            next_link(&h).as_deref(),
            Some("https://x/api/v1/courses?page=2")
        );
    }

    #[test]
    fn handles_unquoted_rel_and_split_headers() {
        let h = headers(&[
            "<https://x/a?page=1>; rel=first",
            "<https://x/a?page=3>;rel=next",
        ]);
        assert_eq!(next_link(&h).as_deref(), Some("https://x/a?page=3"));
    }

    #[test]
    fn none_on_last_page_or_missing_header() {
        let last =
            headers(&[r#"<https://x/a?page=1>; rel="first", <https://x/a?page=3>; rel="last""#]);
        assert_eq!(next_link(&last), None);
        assert_eq!(next_link(&HeaderMap::new()), None);
    }
}
