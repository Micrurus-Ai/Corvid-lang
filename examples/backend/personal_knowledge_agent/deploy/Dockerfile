FROM rust:1.78-slim AS build
WORKDIR /workspace
COPY . .
RUN cargo build -p corvid-cli --release

FROM debian:bookworm-slim
WORKDIR /workspace
COPY --from=build /workspace/target/release/corvid /usr/local/bin/corvid
COPY examples/backend/personal_knowledge_agent examples/backend/personal_knowledge_agent
COPY std std
EXPOSE 8080
CMD ["corvid", "serve", "examples/backend/personal_knowledge_agent/src/main.cor", "--listen", "0.0.0.0:8080"]
