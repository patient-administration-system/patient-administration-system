//! Checked arithmetic helpers (`spec/rust-checked-arithmetic`): sums that report overflow instead of panicking
//! in a debug build or wrapping in a release build, which is what `Iterator::sum` does (RCA-AGG-1).

/// An integer type whose addition can be checked.
pub trait CheckedInt: Copy {
    /// The additive identity.
    const ZERO: Self;
    /// `self + other`, or `None` when it does not fit.
    fn add_checked(self, other: Self) -> Option<Self>;
}

macro_rules! checked_int {
    ($($t:ty),*) => {$(
        impl CheckedInt for $t {
            const ZERO: Self = 0;
            fn add_checked(self, other: Self) -> Option<Self> {
                self.checked_add(other)
            }
        }
    )*};
}
checked_int!(i32, i64, u32, u64, usize);

/// The sum of `items`, or `None` when it does not fit the type. An empty list sums to zero.
pub fn checked_sum<T: CheckedInt>(items: impl IntoIterator<Item = T>) -> Option<T> {
    items
        .into_iter()
        .try_fold(T::ZERO, |total, item| total.add_checked(item))
}

/// `value`, when it is within `lo ..= hi`; otherwise a validation error naming the figure and its range
/// (RCA-IN-1, RCA-IN-2). An input outside its range is rejected: it is never moved to the nearest bound.
/// The error does not carry the value (RCA-ERR-2).
pub fn in_range<T: PartialOrd + std::fmt::Display>(
    figure: &str,
    value: T,
    lo: T,
    hi: T,
) -> crate::Result<T> {
    if value >= lo && value <= hi {
        Ok(value)
    } else {
        Err(crate::Error::validation(format!(
            "{figure} must be from {lo} to {hi}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sum_that_fits_is_exact_and_one_that_does_not_is_none() {
        assert_eq!(checked_sum([1_i64, 2, 3]), Some(6));
        assert_eq!(checked_sum(Vec::<u64>::new()), Some(0));
        assert_eq!(checked_sum([u8_max_as_u32(), 1]), Some(256));
        assert_eq!(checked_sum([i64::MAX, 1]), None);
        assert_eq!(checked_sum([i64::MIN, -1]), None);
        assert_eq!(checked_sum([usize::MAX, 1]), None);
        assert_eq!(checked_sum([i32::MAX, 0]), Some(i32::MAX));
    }

    fn u8_max_as_u32() -> u32 {
        u32::from(u8::MAX)
    }

    #[test]
    fn a_value_outside_its_range_is_refused_naming_the_figure_and_the_range() {
        assert_eq!(in_range("days", 1, 1, 365).unwrap(), 1);
        assert_eq!(in_range("days", 365, 1, 365).unwrap(), 365);
        for bad in [0, 366, -1, i64::MIN, i64::MAX] {
            let e = in_range("days", bad, 1, 365).unwrap_err().to_string();
            assert!(e.contains("days") && e.contains("1 to 365"), "{e}");
            assert!(
                !e.contains(&bad.to_string()) || bad == 1,
                "no operand in {e}"
            );
        }
    }
}
