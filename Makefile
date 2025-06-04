# Makefile for pulse Rust crate

.PHONY: build build-release run run-release clean

build:
	cargo build

build-release:
	cargo build --release

run:
	cargo run

run-release:
	cargo run --release

clean:
	cargo clean
