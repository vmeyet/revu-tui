//! Which way a move goes through a list that wraps around.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    Next,
    Previous,
}

impl Toward {
    pub fn next_if(next: bool) -> Self {
        if next { Self::Next } else { Self::Previous }
    }

    /// The index `steps` away from `from` this way, in a list of `len` that wraps around.
    pub fn wrapped(self, from: usize, steps: usize, len: usize) -> usize {
        match self {
            Self::Next => (from + steps) % len,
            Self::Previous => (from + len - steps % len) % len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_goes_past_the_end_to_the_start() {
        assert_eq!(Toward::Next.wrapped(3, 2, 4), 1);
    }

    #[test]
    fn previous_goes_past_the_start_to_the_end() {
        assert_eq!(Toward::Previous.wrapped(0, 1, 4), 3);
    }

    #[test]
    fn a_whole_round_back_ends_where_it_started() {
        assert_eq!(Toward::Previous.wrapped(2, 4, 4), 2);
    }
}
