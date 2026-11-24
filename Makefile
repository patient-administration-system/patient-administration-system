.PHONY: ci-minimum-supported-rust-version test-benchmarks test-coverage

ci-minimum-supported-rust-version:
	bin/make-ci-minimum-supported-rust-version

test-benchmarks:
	bin/test-benchmarks

test-coverage:
	bin/test-coverage
