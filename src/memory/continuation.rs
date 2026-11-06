//! Public text continuity only. Native tool arguments are never concatenated.
//! Overlap is deliberately conservative: exact repeated segments or a matching
//! suffix of at least 32 characters, bounded to 2048 characters of lookbehind.
#[derive(Default)]
pub(crate) struct Output {
    pub text: String,
    pub segments: usize,
    last: String,
    repeats: usize,
}
impl Output {
    fn overlap(&self, incoming: &str) -> usize {
        if !self.last.is_empty() && incoming.starts_with(&self.last) {
            return self.last.len();
        }
        let mut suffixes = self
            .text
            .char_indices()
            .rev()
            .take(2048)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        suffixes.reverse();
        suffixes
            .into_iter()
            .find_map(|i| {
                let tail = &self.text[i..];
                (tail.chars().count() >= 32 && incoming.starts_with(tail)).then_some(tail.len())
            })
            .unwrap_or(0)
    }
    /// Hold an ambiguous echoed prefix until it diverges or is fully matched,
    /// so already-published text does not flash twice while streaming.
    pub fn visible<'a>(&self, incoming: &'a str, finished: bool) -> &'a str {
        if !finished && self.segments > 0 {
            if self.last.starts_with(incoming) {
                return "";
            }
            if self.text.char_indices().rev().take(2048).any(|(i, _)| {
                let tail = &self.text[i..];
                tail.len() >= 32 && tail.starts_with(incoming)
            }) {
                return "";
            }
        }
        &incoming[self.overlap(incoming)..]
    }
    pub fn commit(&mut self, incoming: &str) -> Result<String, String> {
        let novel = self.visible(incoming, true).to_owned();
        if novel.is_empty() {
            self.repeats += 1;
            if self.repeats >= 2 {
                return Err("Task incomplete: The model repeated the same output segment without advancing. Partial response is preserved.".into());
            }
        } else {
            self.repeats = 0;
            self.text.push_str(&novel);
        }
        self.last = incoming.to_owned();
        self.segments += 1;
        Ok(novel)
    }
    pub fn tail(&self) -> String {
        self.text
            .chars()
            .rev()
            .take(512)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn twenty_segments_preserve_unicode_without_invented_spacing() {
        let mut out = Output::default();
        let mut expected = String::new();
        for n in 0..20 {
            let s = format!("section {n} 🫍\n");
            expected.push_str(&s);
            assert_eq!(out.commit(&s).unwrap(), s);
        }
        assert_eq!(out.text, expected);
    }
    #[test]
    fn echoed_segment_and_partial_overlap_are_not_duplicated_on_stream_or_commit() {
        let mut out = Output::default();
        let s = "This is a long original segment with 🫍 Unicode at the end. ";
        out.commit(s).unwrap();
        assert_eq!(out.visible("This is", false), "");
        assert_eq!(out.commit(s).unwrap(), "");
        assert_eq!(out.commit(&format!("{s}Next part")).unwrap(), "Next part");
        let repeated = "A repeated unchanged segment with enough characters. ";
        let mut out = Output::default();
        out.commit(repeated).unwrap();
        assert!(out.commit(repeated).is_ok());
        assert!(out.commit(repeated).is_err());
        assert_eq!(out.text, repeated);
    }
}
