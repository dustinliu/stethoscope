# Project root Makefile: delegate to pulse/Makefile

PULSE_MAKE := $(MAKE) -C pulse

.PHONY: build build-release run run-release clean

build:
	$(PULSE_MAKE) build

build-release:
	$(PULSE_MAKE) build-release

run:
	$(PULSE_MAKE) run

run-release:
	$(PULSE_MAKE) run-release

clean:
	$(PULSE_MAKE) clean
