# Run example 80 (the Python SDK client).
#
# Stdlib-only: the checked-in finx_platform SDK has no third-party runtime
# dependencies, so a bare python image is enough. The container needs a daemon
# REST surface to talk to — start one separately and pass its URL via
# FINX_BASE_URL (see this example's README), e.g. with host networking:
#
#   docker run --rm --network host -e FINX_BASE_URL=http://127.0.0.1:7879 <image>
FROM python:3.12-slim

WORKDIR /workspace
# The script imports the SDK from sdk/python via a relative path, so copy both
# the SDK package and the example.
COPY sdk/python ./sdk/python
COPY examples/workspace/80-python-sdk ./examples/workspace/80-python-sdk

# Compile-check at build time (the same gate CI runs).
RUN python -m py_compile examples/workspace/80-python-sdk/main.py

ENTRYPOINT ["python", "examples/workspace/80-python-sdk/main.py"]
