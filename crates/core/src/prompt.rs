//! `OSC 133` prompt marks (`M-R`): the substrate shell integration needs (`B39`).
//!
//! The shell says what it is doing — where the prompt starts, where the command
//! starts, where the output starts, when it ends and with what status — and with
//! those four marks the scrollback stops being a wall of lines: a prompt can be
//! jumped to, a command's output can be taken as a range, the label can say what
//! runs. This module keeps only the marks; the consumers live in the app.
//!
//! A mark is stored against an **absolute** line (the same coordinate a
//! selection anchor uses), so scrolling and output do not move it: the text under
//! it moves.

/// Ceiling of the list. A prompt mark is worth a few bytes and a session produces
/// one per command; past this the oldest are dropped, oldest first.
pub const MARK_MAX: usize = 512;

/// The four marks of the protocol (`133;A` … `133;D[;status]`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MarkKind {
    /// `A`: the prompt starts on this line.
    PromptStart,
    /// `B`: the user's input starts (the cursor is at the command prompt).
    CommandStart,
    /// `C`: the command was accepted and its output starts here.
    OutputStart,
    /// `D;<status>`: the command ended, with the status the shell answered.
    CommandEnd,
}

/// One mark, on an absolute line.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Mark {
    pub line: i64,
    pub kind: MarkKind,
    /// Only `D` carries it (the exit status); `None` when the shell sent `D alone
    /// or an unknown status.
    pub status: Option<i32>,
}

/// The marks of one tab, oldest first.
#[derive(Default)]
pub struct Marks {
    list: Vec<Mark>,
}

impl Marks {
    /// Records a mark, dropping what fell out of the history.
    ///
    /// `first` is the first absolute line the buffer still holds (`abs_first`):
    /// a mark below it points at a line nobody can scroll to any more, and a
    /// navigation that landed there would be a jump into nothing.
    pub fn push(&mut self, mark: Mark, first: i64) {
        self.prune(first);
        // an identical mark on the same line is a shell repeating itself (a
        // prompt redrawn after a resize, a `D` sent twice) — not news
        if self.list.last() == Some(&mark) {
            return;
        }
        self.list.push(mark);
        if self.list.len() > MARK_MAX {
            self.list.remove(0);
        }
    }

    /// Drops the marks below `first`.
    pub fn prune(&mut self, first: i64) {
        let n = self.list.iter().take_while(|m| m.line < first).count();
        if n > 0 {
            self.list.drain(..n);
        }
    }

    #[must_use]
    pub fn all(&self) -> &[Mark] {
        &self.list
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `RIS`/`DECSTR` and a fresh shell: the marks belong to the old session.
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// The closest prompt start strictly before `line`, or `None`.
    #[must_use]
    pub fn prompt_before(&self, line: i64) -> Option<i64> {
        self.prompts().rev().find(|&&l| l < line).copied()
    }

    /// The closest prompt start strictly after `line`, or `None`.
    #[must_use]
    pub fn prompt_after(&self, line: i64) -> Option<i64> {
        self.prompts().find(|&&l| l > line).copied()
    }

    fn prompts(&self) -> impl DoubleEndedIterator<Item = &i64> {
        self.list
            .iter()
            .filter(|m| m.kind == MarkKind::PromptStart)
            .map(|m| &m.line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(line: i64, kind: MarkKind) -> Mark {
        Mark {
            line,
            kind,
            status: None,
        }
    }

    #[test]
    fn the_marks_of_a_command_are_kept_in_order() {
        let mut m = Marks::default();
        m.push(mark(3, MarkKind::PromptStart), 0);
        m.push(mark(3, MarkKind::CommandStart), 0);
        m.push(mark(4, MarkKind::OutputStart), 0);
        m.push(
            Mark {
                line: 9,
                kind: MarkKind::CommandEnd,
                status: Some(1),
            },
            0,
        );
        let kinds: Vec<_> = m.all().iter().map(|k| k.kind).collect();
        assert_eq!(
            kinds,
            [
                MarkKind::PromptStart,
                MarkKind::CommandStart,
                MarkKind::OutputStart,
                MarkKind::CommandEnd
            ]
        );
        assert_eq!(m.all()[3].status, Some(1));
    }

    #[test]
    fn a_repeated_mark_is_not_recorded_twice() {
        let mut m = Marks::default();
        m.push(mark(3, MarkKind::PromptStart), 0);
        m.push(mark(3, MarkKind::PromptStart), 0);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn navigation_walks_prompt_to_prompt() {
        let mut m = Marks::default();
        for line in [2, 7, 12] {
            m.push(mark(line, MarkKind::PromptStart), 0);
        }
        m.push(mark(8, MarkKind::CommandStart), 0);
        assert_eq!(m.prompt_before(12), Some(7));
        assert_eq!(m.prompt_before(7), Some(2));
        assert_eq!(m.prompt_before(2), None);
        assert_eq!(m.prompt_after(7), Some(12));
        assert_eq!(m.prompt_after(12), None);
        // a command start is not a prompt: the walk skips it
        assert_eq!(m.prompt_after(7), Some(12));
    }

    #[test]
    fn a_mark_below_the_history_is_dropped() {
        let mut m = Marks::default();
        for line in [1, 5, 9] {
            m.push(mark(line, MarkKind::PromptStart), 0);
        }
        // the scrollback ceiling ate everything below line 5
        m.prune(5);
        assert_eq!(m.all().len(), 2);
        assert_eq!(m.prompt_before(9), Some(5));
        assert_eq!(m.prompt_before(5), None);
    }

    #[test]
    fn the_list_has_a_ceiling() {
        let mut m = Marks::default();
        for line in 0..(MARK_MAX as i64 + 10) {
            m.push(mark(line, MarkKind::PromptStart), 0);
        }
        assert_eq!(m.len(), MARK_MAX);
        // the oldest went first
        assert_eq!(m.all()[0].line, 10);
    }
}
