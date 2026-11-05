use std::io::Write;
fn main() {
    let request = match pkms_worker::read_request(std::io::stdin().lock()) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("{}", error.code);
            std::process::exit(2);
        }
    };
    let response = pkms_worker::execute(&request);
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &response).is_err() || stdout.write_all(b"\n").is_err() {
        std::process::exit(3);
    }
}
