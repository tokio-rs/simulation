fn main() {
    tonic_build::compile_protos("tests/proto/helloworld.proto").unwrap();
}
