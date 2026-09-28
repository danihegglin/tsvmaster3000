//! A single-line text buffer with a byte-offset cursor (always on a char boundary).

#[derive(Default, Clone)]
pub struct LineEdit {
    pub text: String,
    pub cursor: usize,
}

impl LineEdit {
    pub fn new(text: impl Into<String>, at_end: bool) -> Self {
        let text = text.into();
        let cursor = if at_end { text.len() } else { 0 };
        Self { text, cursor }
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn backspace(&mut self) {
        let prev = self.prev_boundary();
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn delete(&mut self) {
        let next = self.next_boundary();
        self.text.replace_range(self.cursor..next, "");
    }

    fn word_start_before(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        match trimmed.chars().next_back() {
            None => 0,
            Some(last) if is_word(last) => trimmed
                .char_indices()
                .rev()
                .find(|&(_, c)| !is_word(c))
                .map_or(0, |(i, c)| i + c.len_utf8()),
            Some(last) => trimmed.len() - last.len_utf8(),
        }
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start_before();
    }

    pub fn word_right(&mut self) {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let rest = &self.text[self.cursor..];
        let mut chars = rest.char_indices().skip_while(|&(_, c)| !is_word(c));
        let skip = chars
            .find(|&(_, c)| !is_word(c))
            .map_or(rest.len(), |(i, _)| i);
        self.cursor += skip;
    }

    pub fn delete_word_before(&mut self) {
        let start = self.word_start_before();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn delete_to_start(&mut self) {
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    pub fn delete_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing() {
        let mut e = LineEdit::new("héllo world", true);
        e.delete_word_before();
        assert_eq!(e.text, "héllo ");
        e.backspace();
        e.left();
        e.backspace();
        assert_eq!(e.text, "hélo");
        e.home();
        e.word_right();
        assert_eq!(e.cursor, e.text.len());
    }
}
