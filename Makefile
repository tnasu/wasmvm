.PHONY: build build-rust build-go

build: build-rust build-go

build-rust:
	cargo build --release
	cp target/release/libgo_cosmwasm.so api

build-go:
	go build .