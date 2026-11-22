# Arithmetic

Maths must not panic. Overflow, underflow, division by zero and an out-of-range conversion are ordinary inputs
for a system that reads numbers from the outside world, so each is handled and none stops the server.

## Requirements

1. **Use Rust's `checked_*` functions** for integer arithmetic whose result could leave the type's range:
   `checked_add`, `checked_sub`, `checked_mul`, `checked_div`, `checked_rem`, `checked_neg`, `checked_abs`,
   `checked_pow`, `checked_shl` and `checked_shr`.
2. **Return `None` instead of panicking or wrapping.** A checked function returns `Option<T>`; the caller
   decides what `None` means (a refusal, a validation error, a default that is safe) and says so where it
   decides. The operators `+`, `-`, `*`, `/`, `%` and `-x` panic on overflow in a debug build and **wrap
   silently in a release build**, which gives a wrong answer with no error. Neither is acceptable.
3. **You must not unwrap the result of a `checked_*` call.** `x.checked_add(y).unwrap()` and
   `.expect("...")` turn the `None` back into the panic the check was there to prevent, so they are no better
   than `x + y`; `unwrap_unchecked` is worse (undefined behaviour). Handle the `None`: return it (`?` in a
   function that returns `Option`), turn it into an error (`.ok_or_else(|| Error::validation(..))?`), or match
   on it. `unwrap_or(default)` is allowed only where that default is safe for the caller, and a comment says
   why. A reason that the check "cannot fail" is not a reason: write the `None` arm.
4. **Convert with `try_from`, not `as`.** `u8::try_from(n)` returns an error when `n` does not fit; `n as u8`
   truncates without a sign.
5. **Do not index or slice** with a computed position: use `.get(i)` or `.get(a..b)`, which return `None` when
   the position is out of range. This is the same rule for the same reason, and is enforced by the
   `clippy::indexing_slicing` and `clippy::string_slice` lints.

## Example

```rust
assert_eq!(255_u8.checked_add(1), None);      // overflow: None, not a panic and not 0
assert_eq!(250_u8.checked_add(5), Some(255));
assert_eq!(1_i32.checked_div(0), None);       // division by zero: None
assert_eq!(i32::MIN.checked_abs(), None);     // |i32::MIN| does not fit
assert!(u8::try_from(256_i32).is_err());      // `256 as u8` would be 0
```

Handling the `None`, not unwrapping it:

```rust
fn add_charge(total: i64, charge: i64) -> Result<i64, Error> {
    total
        .checked_add(charge)
        .ok_or_else(|| Error::validation("the total is too large to hold"))
}

// Wrong: the panic is back, only later.
// total.checked_add(charge).unwrap()
```

`Option<T>` is the type of every checked result: `255_u8.checked_add(1)` is `None`.

## Where

- The rule applies to the server, the domain crate and the front end: all production code, not tests.
- Saturating arithmetic (`saturating_add` and the like) is allowed only where clamping to the limit is the
  intended answer, such as a counter that stops at its maximum; say so in a comment.
- Floating-point arithmetic does not panic, but a result of `NaN` or infinity is an error to check for before
  it is stored or sent.

## How it is checked

`clippy::panic`, `unreachable`, `todo`, `unimplemented`, `indexing_slicing`, `string_slice`, `unwrap_used` and
`expect_used` are denied (`unwrap_used` and `expect_used` are what stop an unwrapped `checked_*` result) in production code by each crate root, so `CI` fails on a new panic path
([`patient-administration-system-with-rust/src/lib.rs`](../../patient-administration-system-with-rust/src/lib.rs)).
Money is a decimal that rejects overflow (`Money::try_add`), not a float.

**Not yet enforced:** `clippy::arithmetic_side_effects`, which would flag every unchecked `+` and `*`. Turning
it on is a separate piece of work: it reports hundreds of sites that each need a decision, so the rule above
is the standard for new code and for what is touched, until that lint is switched on.
