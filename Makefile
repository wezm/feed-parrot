

help:
	@echo "Available tasks:"
	@echo
	@echo "- pizero - build for Raspberry Pi Zero with cross"


pizero:
	cross build --release --target arm-unknown-linux-gnueabihf

.PHONY: pizero
