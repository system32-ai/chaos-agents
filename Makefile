VERSION ?= $(shell grep '^version' crates/chaos-cli/Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
BINARY := chaos
RELEASE_DIR := target/release-artifacts

TARGETS := \
	x86_64-unknown-linux-gnu \
	x86_64-unknown-linux-musl \
	aarch64-unknown-linux-gnu \
	aarch64-unknown-linux-musl \
	x86_64-apple-darwin \
	aarch64-apple-darwin

IMAGE := chaos-agents
.PHONY: build check test clean install list-skills validate-example docker docker-run release-build release release-dry-run

build:
	cargo build --release

check:
	cargo check --workspace

test:
	cargo test --workspace

clean:
	cargo clean
	rm -rf $(RELEASE_DIR)

install:
	cargo install --path crates/chaos-cli

list-skills:
	cargo run --bin chaos -- list-skills

validate-example:
	cargo run --bin chaos -- validate config/example-db.yaml

docker:
	docker build -t $(IMAGE):$(VERSION) -t $(IMAGE):latest .

docker-run:
	docker run --rm $(IMAGE):latest --help

HOST_TARGET := $(shell rustc -vV | grep '^host:' | cut -d' ' -f2)

release-build:
	@mkdir -p $(RELEASE_DIR)
	@for target in $(TARGETS); do \
		echo "Building $$target..."; \
		if [ "$$target" = "$(HOST_TARGET)" ]; then \
			cargo build --release --target $$target -p chaos-cli; \
		else \
			cross build --release --target $$target -p chaos-cli; \
		fi && \
		tar -czf $(RELEASE_DIR)/$(BINARY)-$(VERSION)-$$target.tar.gz \
			-C target/$$target/release $(BINARY) || \
		echo "WARN: Failed to build $$target, skipping"; \
	done
	@echo "Artifacts in $(RELEASE_DIR):"
	@ls -lh $(RELEASE_DIR)/

release-build-local:
	@mkdir -p $(RELEASE_DIR)
	@echo "Building $(HOST_TARGET)..."
	@cargo build --release --target $(HOST_TARGET) -p chaos-cli && \
		tar -czf $(RELEASE_DIR)/$(BINARY)-$(VERSION)-$(HOST_TARGET).tar.gz \
			-C target/$(HOST_TARGET)/release $(BINARY)
	@echo "Artifacts in $(RELEASE_DIR):"
	@ls -lh $(RELEASE_DIR)/

release-dry-run: release-build
	@echo ""
	@echo "Would create GitHub release v$(VERSION) with:"
	@ls $(RELEASE_DIR)/

release: release-build
	gh release create v$(VERSION) \
		--title "v$(VERSION)" \
		--generate-notes \
		$(RELEASE_DIR)/*.tar.gz
