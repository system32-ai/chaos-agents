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

.PHONY: build test clean release-build release release-dry-run

build:
	cargo build --release

test:
	cargo test --workspace

clean:
	cargo clean
	rm -rf $(RELEASE_DIR)

release-build:
	@mkdir -p $(RELEASE_DIR)
	@for target in $(TARGETS); do \
		echo "Building $$target..."; \
		cross build --release --target $$target -p chaos-cli && \
		tar -czf $(RELEASE_DIR)/$(BINARY)-$(VERSION)-$$target.tar.gz \
			-C target/$$target/release $(BINARY) || \
		echo "WARN: Failed to build $$target, skipping"; \
	done
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
