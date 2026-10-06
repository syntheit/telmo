//! Prints every update of one real speedtest.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    telmo_speed::run(|update| println!("{update:?}")).await;
}
