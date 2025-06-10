# Makefile for running a fast local LLM with Ollama and configuring DSPy

# Name of the model (change to your preferred one)
OLLAMA_MODEL ?= stablelm-zephyr:latest

.PHONY: ollama-install ollama-pull ollama-run ollama-status run-host-venv

ollama-install:
	brew install ollama

ollama-pull:
	ollama pull $(OLLAMA_MODEL)

ollama-run:
	ollama run $(OLLAMA_MODEL)

ollama-status:
	curl -s http://localhost:11434/api/tags | jq

run-host-venv:
	PYTHON_BIN=rustysnake/host/pyenv/bin/python3.13; \
	PYTHONPATH=`$$PYTHON_BIN -c 'import site; print(site.getsitepackages()[0])'` \
	PATH=rustysnake/host/pyenv/bin:$$PATH \
	DYLD_LIBRARY_PATH=/opt/homebrew/opt/python@3.13/Frameworks/Python.framework/Versions/3.13/lib:$$DYLD_LIBRARY_PATH \
	cargo run -p host

# Example usage:
#   make ollama-install
#   make ollama-pull OLLAMA_MODEL=phi3:latest
#   make ollama-run OLLAMA_MODEL=phi3:latest
#   make ollama-status

# For highly quantized models, you can specify e.g. phi3:8b or mistral:7b-q4_K_M
# See https://ollama.com/library for available models and quantizations 