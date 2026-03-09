/// Placeholder processing result
pub struct PlaceholderResult {
    /// Text after placeholder removal
    pub text: String,
    /// Cursor position (start position of the first placeholder)
    pub cursor: usize,
}

/// Remove first {{...}} placeholder and move cursor there.
/// Takes ownership of the input string to avoid allocation when no placeholder exists.
pub fn apply_first_placeholder(text: String, default_cursor: usize) -> PlaceholderResult {
    if let Some(start) = text.find("{{") {
        if let Some(end) = text[start..].find("}}") {
            let end_pos = start + end + 2;
            let mut result = String::with_capacity(text.len() - (end_pos - start));
            result.push_str(&text[..start]);
            result.push_str(&text[end_pos..]);
            return PlaceholderResult {
                text: result,
                cursor: start,
            };
        }
    }

    // Zero-cost move instead of allocation
    PlaceholderResult {
        text,
        cursor: default_cursor,
    }
}

/// Find next placeholder (from cursor position onward)
pub fn find_next_placeholder(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let search_start = cursor.min(text.len());
    if let Some(offset) = text[search_start..].find("{{") {
        let start = search_start + offset;
        if let Some(end_offset) = text[start..].find("}}") {
            return Some((start, start + end_offset + 2));
        }
    }

    // If not found after cursor, wrap around from beginning
    if cursor > 0 {
        if let Some(start) = text.find("{{") {
            if start < cursor {
                if let Some(end_offset) = text[start..].find("}}") {
                    return Some((start, start + end_offset + 2));
                }
            }
        }
    }

    None
}

/// Remove all placeholders from text
pub fn remove_all_placeholders(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find("{{") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("}}") {
            remaining = &remaining[start + end + 2..];
        } else {
            // If no closing tag, keep {{ and everything after it
            result.push_str(&remaining[start..]);
            return result;
        }
    }

    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_first_placeholder_basic() {
        let result = apply_first_placeholder("git commit -m '{{message}}'".to_string(), 100);
        assert_eq!(result.text, "git commit -m ''");
        assert_eq!(result.cursor, 15);
    }

    #[test]
    fn test_apply_first_placeholder_at_start() {
        let result = apply_first_placeholder("{{cmd}} arg1 arg2".to_string(), 100);
        assert_eq!(result.text, " arg1 arg2");
        assert_eq!(result.cursor, 0);
    }

    #[test]
    fn test_apply_first_placeholder_at_end() {
        let result = apply_first_placeholder("echo {{value}}".to_string(), 100);
        assert_eq!(result.text, "echo ");
        assert_eq!(result.cursor, 5);
    }

    #[test]
    fn test_apply_first_placeholder_multiple() {
        let result = apply_first_placeholder("{{a}} and {{b}}".to_string(), 100);
        assert_eq!(result.text, " and {{b}}");
        assert_eq!(result.cursor, 0);
    }

    #[test]
    fn test_apply_first_placeholder_none() {
        let result = apply_first_placeholder("no placeholder here".to_string(), 10);
        assert_eq!(result.text, "no placeholder here");
        assert_eq!(result.cursor, 10);
    }

    #[test]
    fn test_apply_first_placeholder_unclosed() {
        let result = apply_first_placeholder("text {{unclosed".to_string(), 5);
        assert_eq!(result.text, "text {{unclosed");
        assert_eq!(result.cursor, 5);
    }

    #[test]
    fn test_find_next_placeholder() {
        let text = "{{a}} and {{b}} and {{c}}";
        let result = find_next_placeholder(text, 0);
        assert_eq!(result, Some((0, 5)));

        let result = find_next_placeholder(text, 5);
        assert_eq!(result, Some((10, 15)));

        let result = find_next_placeholder(text, 15);
        assert_eq!(result, Some((20, 25)));

        // Wrap around
        let result = find_next_placeholder(text, 25);
        assert_eq!(result, Some((0, 5)));
    }

    #[test]
    fn test_find_next_placeholder_none() {
        let result = find_next_placeholder("no placeholder", 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_remove_all_placeholders() {
        assert_eq!(
            remove_all_placeholders("git commit -m '{{message}}' --author='{{author}}'"),
            "git commit -m '' --author=''"
        );
        assert_eq!(remove_all_placeholders("no placeholders"), "no placeholders");
        assert_eq!(remove_all_placeholders("{{only}}"), "");
    }

    #[test]
    fn test_remove_all_placeholders_unclosed() {
        // "text {{valid}} text {{unclosed" -> removes valid, keeps the rest as-is
        assert_eq!(
            remove_all_placeholders("{{valid}} text {{unclosed"),
            " text {{unclosed"
        );
        // Simple case with no closing tag
        assert_eq!(
            remove_all_placeholders("only {{unclosed"),
            "only {{unclosed"
        );
    }

    #[test]
    fn test_placeholder_cursor_within_bounds() {
        // When placeholders exist, cursor should be within text length
        let texts = vec![
            "{{a}}",
            "pre {{a}} post",
            "{{a}} {{b}}",
            "text",
            "",
        ];
        for text in texts {
            let len = text.len();
            let result = apply_first_placeholder(text.to_string(), len);
            assert!(
                result.cursor <= result.text.len(),
                "cursor {} > text.len() {} for input {:?}",
                result.cursor,
                result.text.len(),
                text
            );
        }
    }
}
