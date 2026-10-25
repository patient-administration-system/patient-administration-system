//! A state or kind that is one of a fixed list of words (QUAL-28).
//!
//! `string_enum!` declares the enum with the word each variant is stored and sent as, and gives it
//! `as_str`, `FromStr`, `NAMES` (the list, for the database drift test in `tests/qual28_state_lists_test.rs`),
//! `Serialize`/`Deserialize` and an OpenAPI schema that lists the values. The database keeps its `CHECK`
//! constraint; the drift test fails when the two lists differ.

/// Declare a state enum: `string_enum! { /** doc */ pub enum Name { Variant = "word", ... } }`.
#[macro_export]
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $word:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
        $vis enum $name {
            $( $(#[$vmeta])* #[serde(rename = $word)] $variant ),+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[ $( Self::$variant ),+ ];
            /// Every value as stored and sent.
            pub const NAMES: &'static [&'static str] = &[ $( $word ),+ ];

            /// The stored and sent form.
            pub fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $word ),+ }
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::Error;

            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                match s {
                    $( $word => Ok(Self::$variant), )+
                    _ => Err($crate::Error::validation(format!(
                        "{} must be one of {}",
                        stringify!($name),
                        Self::NAMES.join(", ")
                    ))),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }
    };
}
