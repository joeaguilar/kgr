//! Test-only assertions shared by the kgr crates.
//!
//! Clippy's opinion of how an assertion should be spelled changes between
//! releases (1.99 added `clippy::assert_is_empty`, which flags
//! `assert!(x.is_empty())` and `assert!(!x.is_empty())`). Tests assert
//! through these macros instead, so the next such change is fixed here once
//! rather than at every call site.

/// Asserts that `value.is_empty()`, printing the value when it is not.
///
/// Takes anything with an `is_empty()` method that implements `Debug`:
/// `Vec`, slices, maps, sets, `String`, `&str`. A trailing format message is
/// added to the panic, as with `assert!`.
///
/// ```
/// use kgr_test_support::assert_empty;
///
/// let unresolved: Vec<&str> = Vec::new();
/// assert_empty!(unresolved);
/// assert_empty!(unresolved, "while resolving {}", "src/main.rs");
/// ```
#[macro_export]
macro_rules! assert_empty {
    ($value:expr $(,)?) => {
        // `match` rather than `let` keeps temporaries in `$value` alive for
        // the whole assertion, as `assert_eq!` does.
        match &$value {
            value => ::std::assert!(
                value.is_empty(),
                "assertion `{}.is_empty()` failed\n  value: {:?}",
                ::std::stringify!($value),
                value,
            ),
        }
    };
    ($value:expr, $($arg:tt)+) => {
        match &$value {
            value => ::std::assert!(
                value.is_empty(),
                "assertion `{}.is_empty()` failed: {}\n  value: {:?}",
                ::std::stringify!($value),
                ::std::format_args!($($arg)+),
                value,
            ),
        }
    };
}

/// Asserts that `!value.is_empty()`, printing the value when it is empty.
///
/// The counterpart of [`assert_empty!`], taking the same arguments.
///
/// ```
/// use kgr_test_support::assert_not_empty;
///
/// assert_not_empty!(vec!["src/main.rs"]);
/// assert_not_empty!("kgr", "binary name for {}", "linux");
/// ```
#[macro_export]
macro_rules! assert_not_empty {
    ($value:expr $(,)?) => {
        match &$value {
            value => ::std::assert!(
                !value.is_empty(),
                "assertion `!{}.is_empty()` failed\n  value: {:?}",
                ::std::stringify!($value),
                value,
            ),
        }
    };
    ($value:expr, $($arg:tt)+) => {
        match &$value {
            value => ::std::assert!(
                !value.is_empty(),
                "assertion `!{}.is_empty()` failed: {}\n  value: {:?}",
                ::std::stringify!($value),
                ::std::format_args!($($arg)+),
                value,
            ),
        }
    };
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    #[test]
    fn passes_for_empty_values_of_any_collection_type() {
        assert_empty!(Vec::<u8>::new());
        assert_empty!([0u8; 0]);
        assert_empty!(String::new());
        assert_empty!("");
        assert_empty!(BTreeMap::<u8, u8>::new());
        assert_empty!(HashSet::<u8>::new(), "with a {} message", "custom");
    }

    #[test]
    #[should_panic(expected = "assertion `items.is_empty()` failed\n  value: [1, 2]")]
    fn failure_names_the_expression_and_prints_the_value() {
        let items = vec![1, 2];
        assert_empty!(items);
    }

    #[test]
    #[should_panic(expected = "failed: while checking main.rs\n  value: \"x\"")]
    fn failure_includes_the_custom_message() {
        assert_empty!("x", "while checking {}", "main.rs");
    }

    #[test]
    fn temporaries_in_the_expression_live_through_the_assertion() {
        assert_empty!(String::from("  ").trim());
        assert_not_empty!(String::from(" x ").trim());
    }

    #[test]
    fn not_empty_passes_for_non_empty_values() {
        assert_not_empty!(vec![1]);
        assert_not_empty!("x");
        assert_not_empty!(BTreeMap::from([(1, 2)]), "with a {} message", "custom");
    }

    #[test]
    #[should_panic(expected = "assertion `!items.is_empty()` failed\n  value: []")]
    fn not_empty_failure_names_the_expression_and_prints_the_value() {
        let items: Vec<u8> = Vec::new();
        assert_not_empty!(items);
    }

    #[test]
    #[should_panic(expected = "failed: for main.rs\n  value: \"\"")]
    fn not_empty_failure_includes_the_custom_message() {
        assert_not_empty!("", "for {}", "main.rs");
    }
}
