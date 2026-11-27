# Using the Python SDK in FIPS environments

This guide shows how to run the Data Connect Hub Python SDK so that its TLS connections use the host's FIPS-validated cryptography: which images to use, how to install the SDK, and how to verify and enforce the result.

## How the SDK uses cryptography

The SDK implements no cryptography. Its TLS connections run in the libraries of the Python environment it's installed into:

| Traffic | Library | Uses the host's FIPS-validated OpenSSL when |
|---|---|---|
| REST (connections, connection types) | `httpx` and Python's `ssl` module | Python links the system OpenSSL, as on RHEL-based images, and the host is in FIPS mode |
| Flight SQL (queries, tables, server info) | `pyarrow.flight` (gRPC) | pyarrow comes from the Red Hat AI Python index, whose build links the system OpenSSL |

PyPI's pyarrow compiles its own OpenSSL into its libraries and never uses the host's FIPS provider, even on a FIPS-mode host. It can't be fixed after installation, so a FIPS setup must install pyarrow from the Red Hat AI Python index.

## Supported setup

- An OpenShift cluster with its nodes in FIPS mode.
- A Red Hat Python 3.12 image: an RHOAI workbench image, or another RHOAI or AIPCC Python 3.12 image. See [Use a Red Hat image](#use-a-red-hat-image).
- The SDK's dependencies installed from the Red Hat AI Python index. See [Install the SDK](#install-the-sdk).
- SDK 0.3.0 or later. Earlier versions depend on the ADBC Flight SQL driver, which uses Go's TLS and has no wheels on the Red Hat index.

Red Hat pyarrow needs system libraries that these images ship and plain UBI images don't, such as `libthrift`, `libre2` and `libsnappy`. On a plain `ubi9/python-312` image, `import pyarrow` fails with an error like `ImportError: libthrift-0.24.0.so: cannot open shared object file`.

## Use a Red Hat image

### RHOAI workbench

In the Red Hat OpenShift AI dashboard, create a workbench with a Python 3.12 image, for example **Jupyter | Data Science | CPU | Python 3.12**, and open a terminal or notebook in it.

Workbench images are ready for a FIPS install:

- They point pip and uv at the Red Hat AI Python index instead of PyPI, through `PIP_CONFIG_FILE=/opt/app-root/pip.conf` and `UV_CONFIG_FILE=/opt/app-root/uv.toml`.
- They ship the system libraries that Red Hat pyarrow needs.
- The Data Science images come with Red Hat pyarrow and pandas installed.

To confirm the index configuration:

```console
$ pip config list
global.index-url='https://packages.redhat.com/api/pypi/public-rhai/cpu-torch2.13-el9.8/simple/'
```

The index URL depends on the image version.

### Pods, jobs and your own images

Outside the dashboard, use the same workbench images. On a cluster with RHOAI installed, list the images for the installed version with:

```console
oc get imagestream -n redhat-ods-applications
```

and use an image reference from one of them, for example:

```console
oc run dch-client -n <namespace> --restart=Never --command \
  --image=registry.redhat.io/rhoai/odh-workbench-jupyter-datascience-cpu-py312-rhel9@sha256:<digest> \
  -- sleep infinity
```

To build your own image, start from a workbench or AIPCC Python 3.12 image so that the system libraries and the index configuration carry over:

```dockerfile
FROM registry.redhat.io/rhoai/odh-workbench-jupyter-datascience-cpu-py312-rhel9@sha256:<digest>
RUN pip install --no-cache-dir "data-connect-hub[flight]>=0.3.0"
```

Don't override `PIP_INDEX_URL` or `PIP_CONFIG_FILE` in the image. If a base image doesn't configure the Red Hat AI Python index, pass it with `--index-url`, as shown in [Install the SDK](#install-the-sdk).

## Install the SDK

In a workbench image, install as usual. pip and uv already use the Red Hat AI Python index:

```console
pip install "data-connect-hub[flight]"
```

Install only `data-connect-hub` for REST-only use. REST doesn't need pyarrow.

In an image without the index configuration, name the index explicitly. Use the index URL that matches your image:

```console
pip install --index-url https://packages.redhat.com/api/pypi/public-rhai/cpu-torch2.13-el9.8/simple/ \
  "data-connect-hub[flight]"
```

Don't add PyPI as a second index. pip picks the highest version from any configured index, so PyPI's pyarrow can replace the Red Hat build. These all bring it back:

- `--extra-index-url https://pypi.org/simple`, or `PIP_INDEX_URL` or `PIP_EXTRA_INDEX_URL` pointing at PyPI
- `uv pip install --index https://pypi.org/simple`, or a uv project index pointing at PyPI
- `pip install -U pyarrow`, or a pyarrow version pin, while PyPI is configured
- Requirements or lock files generated against PyPI

### New SDK releases and the Red Hat index

New SDK releases reach PyPI first and the Red Hat AI Python index later. Until a release arrives there, a workbench installs the previous release. To use a new release sooner, install only the SDK from PyPI and keep its dependencies on the Red Hat index. The SDK is pure Python and contains no cryptography:

```console
pip install "data-connect-hub[flight]"   # dependencies, and the previous SDK, from the Red Hat index
pip install --no-deps --index-url https://pypi.org/simple "data-connect-hub==<version>"
pip check                                # flags dependencies the Red Hat index doesn't have yet
```

Then [verify](#verify-the-setup) the setup.

In a uv project, pin only the SDK to PyPI. Use the index URL from your image's `/opt/app-root/uv.toml`:

```toml
[[tool.uv.index]]
name = "redhat"
url = "https://packages.redhat.com/api/pypi/public-rhai/cpu-torch2.13-el9.8/simple/"
default = true

[[tool.uv.index]]
name = "pypi"
url = "https://pypi.org/simple"
explicit = true

[tool.uv.sources]
data-connect-hub = { index = "pypi" }
```

## Verify the setup

`fips_status()` inspects the running process and makes no network calls:

```python
from data_connect_hub import fips_status

print(fips_status())
# FipsStatus(host_fips=True, rest_tls='fips', flight_system_openssl=True, pyarrow_version='25.0.1', pyarrow_error=None)
```

| Field | Meaning |
|---|---|
| `host_fips` | The kernel is in FIPS mode (`/proc/sys/crypto/fips_enabled` is `1`) |
| `rest_tls` | `"fips"`: Python uses the system OpenSSL 3 with the FIPS provider enforced. `"not-fips"`: it doesn't, or Python carries its own OpenSSL, as uv-managed Pythons do. `"unknown"`: not determinable, for example on macOS |
| `flight_system_openssl` | `True`: `pyarrow.flight` uses the system OpenSSL. `False`: pyarrow carries its own OpenSSL, as PyPI builds do. `None`: pyarrow is missing or fails to load, or the OS isn't Linux |
| `pyarrow_version` | Installed pyarrow version, or `None` |
| `pyarrow_error` | Why `pyarrow.flight` failed to import, for example a missing `libthrift` outside an RHOAI or AIPCC image |

A FIPS-compliant setup reports `host_fips=True` and `rest_tls="fips"`. If you use Flight SQL, it must also report `flight_system_openssl=True`; a REST-only install without pyarrow reports `None`.

## Enforce FIPS

Pass `require_fips=True`, or set `DCH_REQUIRE_FIPS=1` in the environment, to make the client fail instead of running TLS outside the FIPS module:

```python
from data_connect_hub import DataConnectClient

client = DataConnectClient(
    endpoint="dch.example.com:443",
    token_provider=get_token,
    tenant_id="my-tenant",
    ca_cert="/path/to/cluster-ca.pem",
    require_fips=True,
)
```

An explicit `require_fips` argument overrides the environment variable, which accepts `1`, `true`, `yes` or `on`, and `0`, `false`, `no` or `off`.

| Check | When | If it fails |
|---|---|---|
| The host is in FIPS mode, and REST TLS is `"fips"` | Creating the client | `DCHConfigError` listing each failed check |
| `pyarrow.flight` uses the system OpenSSL | The first Flight call | `DCHConfigError` |
| `insecure=True` | Creating the client | `DCHFipsWarning`: the crypto stays in the FIPS module, but server certificates aren't verified |

Creating the client doesn't import pyarrow, so REST-only installs work with `require_fips`. Without `require_fips`, the SDK doesn't check FIPS.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `host_fips=False` | The node isn't in FIPS mode | Run on a FIPS-enabled cluster |
| `rest_tls='not-fips'` | Python carries its own OpenSSL, as uv-managed and python.org builds do, or the FIPS provider isn't enforced | Use the image's own `python3`, not a separately installed Python |
| `flight_system_openssl=False` | pyarrow came from PyPI | Reinstall it from the Red Hat index: `pip install --force-reinstall --no-deps pyarrow` with only that index configured, or start from a fresh workbench |
| `pyarrow_error` mentions `libthrift`, `libre2` or `libsnappy` | Red Hat pyarrow on an image without its system libraries | Use an RHOAI workbench or AIPCC image |
| `DCHConfigError: pyarrow is installed but failed to load` | Same as above | Same as above |
| `ResolutionImpossible` mentioning `adbc-driver-flightsql` | An SDK version earlier than 0.3.0 | Install SDK 0.3.0 or later |

## Limits

- `insecure=True` disables certificate verification, not TLS encryption or the FIPS module. Use `ca_cert` to trust a private CA instead; it changes which CAs are trusted, not which module performs TLS.
- The SDK isn't a validated cryptographic module itself. The validated module is the host's OpenSSL FIPS provider; see Red Hat's RHEL FIPS documentation for its CMVP certificate.
- `fips_status()` and `require_fips` check the process they run in. They don't cover the Data Connect Hub services or the TLS configuration of the server you connect to.
