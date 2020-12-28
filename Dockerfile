FROM clux/muslrust as build

WORKDIR /app/

# Deps caching begins
COPY Cargo.toml .
COPY Cargo.lock .
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs

RUN apt-get update && apt-get install -y pkg-config libssl-dev libpq-dev && apt-get clean

RUN cargo build --release
# Deps caching ends

COPY . .
RUN touch src/main.rs

RUN cargo build --release

FROM alpine
WORKDIR /app/

COPY --from=build app/target/x86_64-unknown-linux-musl/release/gomiws ./

CMD ["./gomiws"]
