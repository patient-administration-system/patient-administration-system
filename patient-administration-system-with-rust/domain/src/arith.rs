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

/// `a + b`, or an internal error naming `figure` when the sum does not fit its type (RCA-ENF-3). The error does
/// not carry the values (RCA-ERR-2).
pub fn add<T: CheckedInt>(figure: &str, a: T, b: T) -> crate::Result<T> {
    a.add_checked(b)
        .ok_or_else(|| crate::Error::unprocessable(format!("{figure} is too large to count")))
}

/// `n` as a float, or `None` when its magnitude is above 2^53 and the float would be rounded (RCA-CONV-4).
pub fn to_f64(n: i64) -> Option<f64> {
    const LIMIT: i64 = 1 << 53;
    if !(-LIMIT..=LIMIT).contains(&n) {
        return None;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "RCA-CONV-4: the magnitude is checked to be at most 2^53 above, so the float is exact"
    )]
    let x = n as f64;
    Some(x)
}

/// `part / of` as a float, or `None` when `of` is not above zero or either figure does not convert exactly
/// (RCA-CONV-4, RCA-NONE-2). Accepts any count type that fits an `i64`.
pub fn ratio<A: TryInto<i64>, B: TryInto<i64>>(part: A, of: B) -> Option<f64> {
    let (part, of) = (part.try_into().ok()?, of.try_into().ok()?);
    if of <= 0 {
        return None;
    }
    Some(to_f64(part)? / to_f64(of)?)
}

/// `value` as another integer type, or a validation error naming `figure` when it does not fit (RCA-CONV-1).
/// The error does not carry the value (RCA-ERR-2).
pub fn narrow<T: TryFrom<S>, S>(figure: &str, value: S) -> crate::Result<T> {
    T::try_from(value).map_err(|_| crate::Error::unprocessable(format!("{figure} is out of range")))
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
