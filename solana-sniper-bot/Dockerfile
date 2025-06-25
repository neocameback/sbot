# ---- Build Stage ----
  FROM rust:1.87 as builder

  WORKDIR /app
  
  # Copy only the manifest first to cache dependencies
  COPY ./ ./
  # COPY Cargo.toml Cargo.lock ./
  # COPY src ./src
  # COPY wallets ./wallets
  # COPY .env ./
  # COPY config.json ./
  
  RUN cargo build --release
  
  # ---- Runtime Stage ----
  FROM debian:bullseye-slim
  
  WORKDIR /app
  
  RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
  
  COPY --from=builder /app/target/release/cli /app/cli
  # COPY --from=builder /app/.env /app/.env
  # COPY --from=builder /app/config.json /app/config.json
  # COPY --from=builder /app/wallets /app/wallets
  
  ENV RUST_LOG=info
  
  # CMD ["./cli", "start"] 