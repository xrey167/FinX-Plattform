fn main() {
    prost_build::compile_protos(&["proto/market_data.proto"], &["proto/"])
        .expect("prost-build: failed to compile market_data.proto");
}
