fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../proto/chamber_agent.proto");
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(&["../proto/chamber_agent.proto"], &["../proto"])?;
    Ok(())
}
