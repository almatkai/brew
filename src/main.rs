#[tokio::main(worker_threads = 2)]
async fn main() {
    if tezbar_extension_sdk::serve(tezbar_brew::Adapter::default())
        .await
        .is_err()
    {
        eprintln!("Brew extension protocol failed");
        std::process::exit(1);
    }
}
