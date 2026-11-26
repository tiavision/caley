FROM rust:1-bookworm AS backend-build

WORKDIR /app/backend
RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl

COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src/ src/
COPY backend/migrations migrations/
RUN cargo build --release --target x86_64-unknown-linux-musl
# The spec of the routes this binary serves, for the frontend build below.
RUN target/x86_64-unknown-linux-musl/release/caley --openapi > /app/openapi.json

FROM node:26-bookworm AS frontend-build

WORKDIR /app/frontend
# patina (../patina) is a local file dependency of the frontend: it must be there before npm ci links it.
COPY patina/ /app/patina/
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
# The pages are checked against the binary's own spec, not the committed copy (which may be stale).
COPY --from=backend-build /app/openapi.json openapi.json
RUN npm run build

FROM scratch

COPY --from=backend-build /app/backend/target/x86_64-unknown-linux-musl/release/caley /caley
COPY --from=frontend-build /app/frontend/dist /static

ENV STATIC_DIR=/static
EXPOSE 3000

ENTRYPOINT ["/caley"]
