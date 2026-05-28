FROM rust:1.78-slim AS build
WORKDIR /workspace
COPY . .
RUN cargo build -p corvid-cli --release

FROM debian:bookworm-slim
WORKDIR /workspace
COPY --from=build /workspace/target/release/corvid /usr/local/bin/corvid
COPY examples/backend/finance_operations_agent examples/backend/finance_operations_agent
COPY std std
EXPOSE 8080
CMD ["corvid", "serve", "examples/backend/finance_operations_agent/src/main.cor", "--listen", "0.0.0.0:8080"]
