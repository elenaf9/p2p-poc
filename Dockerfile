# select image
FROM rust:latest as build

# create a new empty shell project
RUN USER=root cargo new --bin iota-p2p-poc
WORKDIR /iota-p2p-poc

# copy over your manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./build.rs ./build.rs
COPY ./src/dht.proto ./src/dht.proto

# this build step will cache your dependencies
RUN cargo build --release
RUN rm src/*.rs

# copy your source tree
COPY ./src ./src

RUN cargo clean

# build for release
RUN cargo build --release

# our final base
FROM rust:latest

# copy the build artifact from the build stage
COPY --from=build /iota-p2p-poc/target/release/iota-p2p-poc .

# set the startup command to run your binary
CMD ["./iota-p2p-poc"]