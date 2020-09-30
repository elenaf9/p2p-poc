# select image
FROM rust:latest as build

# create a new empty shell project
RUN USER=root cargo new --bin iota-p2p-poc
WORKDIR /iota-p2p-poc

# copy over manifests
# COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./build.rs ./build.rs
COPY ./src/dht.proto ./src/dht.proto

# cache dependencies
RUN cargo build --release --features server
RUN rm src/*.rs

# copy source tree
COPY ./src ./src

RUN cargo clean

# build for release
RUN cargo build --release --features server

# final base
FROM rust:latest

# copy the build artifact from the build stage
COPY --from=build /iota-p2p-poc/target/release/iota-p2p-poc .

# command for starting the container
ENTRYPOINT ["/iota-p2p-poc"]