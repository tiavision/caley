FROM node:22-bookworm AS frontend-build

WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1-bookworm AS backend-build

WORKDIR /app/backend
RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl

COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --target x86_64-unknown-linux-musl && rm -rf src

COPY backend/src/ src/
COPY backend/templates/ templates/
RUN touch src/main.rs && cargo build --release --target x86_64-unknown-linux-musl

FROM scratch

COPY --from=backend-build /app/backend/target/x86_64-unknown-linux-musl/release/caley /caley
COPY --from=frontend-build /app/frontend/dist /static

ENV STATIC_DIR=/static
EXPOSE 3000

ENTRYPOINT ["/caley"]