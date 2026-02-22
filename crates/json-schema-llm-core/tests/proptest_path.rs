//! Property-based tests for JSON Pointer path utilities.
//!
//! Properties under test:
//! 1. escape/unescape roundtrip: `unescape(escape(s)) == s`
//! 2. build/split roundtrip: `split_path(build_path("#", segs)) == segs`
//! 3. split_path determinism: `split_path(p) == split_path(p)`

use json_schema_llm_core::{
    build_path, escape_pointer_segment, split_path, unescape_pointer_segment,
};
use proptest::prelude::*;

/// Generate arbitrary path segments including edge cases:
/// empty strings, `/`, `~`, `~0`, `~1`, numeric-prefixed, and general printable chars.
fn arb_segment() -> impl Strategy<Value = String> {
    prop_oneof![
        // Edge-case literals
        Just("".to_string()),
        Just("/".to_string()),
        Just("~".to_string()),
        Just("~0".to_string()),
        Just("~1".to_string()),
        Just("0".to_string()),
        Just("42items".to_string()),
        // General printable strings (including `/` and `~`)
        "[[:print:]]{0,30}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..Default::default() })]

    /// Property: `unescape(escape(s)) == s` for arbitrary strings.
    #[test]
    fn escape_unescape_roundtrip(s in arb_segment()) {
        let escaped = escape_pointer_segment(&s);
        let unescaped = unescape_pointer_segment(&escaped);
        prop_assert_eq!(unescaped.as_ref(), s.as_str());
    }

    /// Property: `split_path(build_path("#", segs)) == segs` for arbitrary segment lists.
    #[test]
    fn build_split_roundtrip(segments in proptest::collection::vec(arb_segment(), 0..8)) {
        let refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        let path = build_path("#", &refs);
        let recovered = split_path(&path);
        prop_assert_eq!(recovered, segments);
    }

    /// Property: `split_path` is deterministic (pure function).
    #[test]
    fn split_path_deterministic(segments in proptest::collection::vec(arb_segment(), 0..8)) {
        let refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        let path = build_path("#", &refs);
        let first = split_path(&path);
        let second = split_path(&path);
        prop_assert_eq!(first, second);
    }
}
