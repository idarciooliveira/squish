//! The size guard (ADR-0011): never return a file bigger than the input.

/// Picks the candidate only if it is strictly smaller than the original.
/// The flag is `kept_original`.
pub(crate) fn apply(original: &[u8], candidate: Vec<u8>) -> (Vec<u8>, bool) {
    if candidate.len() < original.len() { (candidate, false) } else { (original.to_vec(), true) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smaller_candidate_wins() {
        assert_eq!(apply(b"original", b"tiny".to_vec()), (b"tiny".to_vec(), false));
    }

    #[test]
    fn equal_and_larger_candidates_lose() {
        assert_eq!(apply(b"abcd", b"wxyz".to_vec()), (b"abcd".to_vec(), true));
        assert_eq!(apply(b"abcd", b"larger".to_vec()), (b"abcd".to_vec(), true));
    }

    #[test]
    fn empty_candidate_is_smaller_than_anything() {
        assert!(!apply(b"a", vec![]).1);
        assert!(apply(b"", vec![]).1);
    }
}
