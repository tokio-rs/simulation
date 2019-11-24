fn main() {
    tonic_build::compile_protos("examples/proto/helloworld.proto").unwrap();
    tonic_build::compile_protos("examples/proto/bank.proto").unwrap();
}
