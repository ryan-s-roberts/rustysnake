PYTHON = /usr/local/bin/python3.13

pyenv:
	$(PYTHON) -m venv rustysnake/host/pyenv

install:
	rustysnake/host/pyenv/bin/pip install -r rustysnake/host/requirements.txt

clean:
	rm -rf rustysnake/host/pyenv

run-host-venv:
	PYTHON_BIN=rustysnake/host/pyenv/bin/python3.13; \
	PYTHONPATH=`$$PYTHON_BIN -c 'import site; print(site.getsitepackages()[0])'` \
	PATH=rustysnake/host/pyenv/bin:$$PATH \
	DYLD_LIBRARY_PATH=/Library/Frameworks/Python.framework/Versions/3.13/lib/:$$DYLD_LIBRARY_PATH \
	PYTHON_GIL=1 \
	cargo run -p host

