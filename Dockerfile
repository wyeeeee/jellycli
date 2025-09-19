# 构建阶段
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

# 运行阶段
FROM alpine:latest

RUN adduser -D -u 1001 jellycli
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/jellycli /app/jellycli

WORKDIR /app
EXPOSE 7878
USER 1001

CMD ["./jellycli"]