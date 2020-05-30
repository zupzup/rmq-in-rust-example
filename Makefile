build:
	@cargo build

clean:
	@cargo clean

TESTS = ""
test:
	@cargo test $(TESTS) --offline --lib -- --color=always --nocapture

docs: build
	@cargo doc --no-deps

style-check:
	@rustup component add rustfmt 2> /dev/null
	cargo fmt --all -- --check

lint:
	@rustup component add clippy 2> /dev/null
	cargo clippy --all-targets --all-features -- -D warnings

rmqstart:
	@sudo docker run -d -p 5672:5672 -p 15672:15672 -e RABBITMQ_DEFAULT_USER=rmq -e RABBITMQ_DEFAULT_PASS=rmq  --name rmqlocal rabbitmq:3.8.4-management

rmqstop:
	@sudo docker stop rmqlocal && sudo docker rm rmqlocal

dev:
	cargo run


.PHONY: build test docs style-check lint 
