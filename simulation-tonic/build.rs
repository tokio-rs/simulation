fn main() {
    tonic_build::compile_protos("tests/proto/helloworld.proto").unwrap();
    tonic_build::compile_protos("tests/proto/bank.proto").unwrap();
}
