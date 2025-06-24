# Solana Sniper Bot

## Overview

A CLI tool for managing and running a Solana Sniper Bot.

## Configuration

All configuration values are loaded from the `.env` file in the project root. See `.env.example` for required variables.

## Running with Docker

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/)
- [Docker Compose](https://docs.docker.com/compose/install/)

### 1. Prepare Environment

- Copy `.env.example` to `.env` and fill in your values.
- Ensure `config.json` and the `wallets/` directory exist in the project root.

### 2. Build and Run

To build and start the bot:

```sh
./scripts/run.sh
```

This will build the Docker image and start the bot using Docker Compose.

### 3. Stop the Bot

To stop the bot:

```sh
./scripts/stop.sh
```

### 4. Running CLI Commands

You can run other CLI commands by overriding the command in Docker Compose:

```sh
docker-compose run --rm sniper-bot ./cli create-wallets --count 5
```

### 5. Persistent Data

- Wallets and configuration are mounted as volumes, so changes persist across container restarts.

## Manual Docker Usage

To build and run manually:

```sh
docker build -t solana-sniper-bot .
docker run --rm -it \
  -v $(pwd)/wallets:/app/wallets \
  -v $(pwd)/.env:/app/.env \
  -v $(pwd)/config.json:/app/config.json \
  solana-sniper-bot
```

## Development

You can still run the bot locally with Cargo:

```sh
cargo run --bin cli -- start
```

---

For more details, see the CLI help:

```sh
cargo run --bin cli -- --help
```
