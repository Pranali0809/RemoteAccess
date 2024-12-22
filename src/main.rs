mod signalling_server;

#[tokio::main]
async fn main() {
    // env_logger::init();
    log4rs::init_file("log4rs.yaml", Default::default()).unwrap();
    signalling_server::signalling_server::main().await;
}