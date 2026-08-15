// Compiles iTerm2's API schema into Rust.
//
// protox is a protobuf compiler written in Rust, so `cargo build` needs no `protoc`
// on the machine. It hands prost-build an already-parsed descriptor set.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/api.proto");

    let fds = protox::compile(["proto/api.proto"], ["proto"])?;
    prost_build::Config::new().compile_fds(fds)?;

    Ok(())
}
