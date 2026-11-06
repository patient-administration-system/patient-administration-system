// `sqlx::migrate!` embeds the files at compile time; a new or changed file must rebuild the crate.
fn main() {
    println!("cargo:rerun-if-changed=sql");
}
