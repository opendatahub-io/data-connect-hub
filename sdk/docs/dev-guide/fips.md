# Python SDK on FIPS-enabled OpenShift AI clusters

The Data Connect Hub Python SDK can run its TLS connections in the host's FIPS-validated OpenSSL. Where that works depends on what you use:

| Use | Works in | Install from |
|---|---|---|
| [REST only](#rest) (connections, connection types, credential tests, binary downloads) | Any FIPS-enabled environment with a RHEL-based Python 3.11 or later, such as a UBI Python pod or an RHOAI workbench | PyPI or the Red Hat AI Python index |
| [REST and Flight SQL](#rest-and-flight-sql) (queries, tables, `read_pandas`) | Only RHOAI workbench or AIPCC images with **Python 3.12** | The Red Hat AI Python index |

Flight SQL needs Python 3.12 because the Red Hat AI Python index publishes pyarrow only for CPython 3.12.

Both need SDK 0.3.0 or later, which adds the FIPS checks below and replaces the ADBC Flight SQL driver, whose Go TLS can't use the host's FIPS module.

For step-by-step examples, see the user guides: [REST on a FIPS cluster](../user-guide/fips-rest.md) and [REST and Flight SQL on FIPS-enabled OpenShift AI clusters](../user-guide/fips-rest-flight.md).

## How the SDK uses cryptography

The SDK implements no cryptography. Its TLS connections run in the libraries of the Python environment it's installed into:

| Traffic | Library | Uses the host's FIPS-validated OpenSSL when |
|---|---|---|
| REST | `httpx` and Python's `ssl` module | Python links the system OpenSSL, as RHEL-based Pythons do, and the host is in FIPS mode |
| Flight SQL | `pyarrow.flight` (gRPC) | pyarrow comes from the Red Hat AI Python index, whose build links the system OpenSSL |

## Verify with `fips_status()`

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
| `pyarrow_error` | Why `pyarrow.flight` failed to import, for example a missing system library outside an RHOAI or AIPCC image |

A FIPS-compliant setup reports `host_fips=True` and `rest_tls="fips"`. If you use Flight SQL, it must also report `flight_system_openssl=True`; a REST-only install without pyarrow reports `None`.

## Enforce with `require_fips`

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

## REST

REST works in any FIPS-enabled environment whose Python, 3.11 or later, links the system OpenSSL. That includes the UBI Python images (`registry.access.redhat.com/ubi9/python-312`), RHEL's `python3` packages and RHOAI workbench images. A Red Hat image isn't required beyond that, and nothing needs to come from the Red Hat AI Python index: the REST install contains no cryptography of its own, so installing from PyPI is fine:

```console
pip install "data-connect-hub>=0.3.0"
```

Pythons that carry their own OpenSSL don't use the host's FIPS module, even on a FIPS-mode host. These include uv-managed Pythons, python.org builds and conda. Use the image's own `python3`.

A REST-only setup reports:

```text
FipsStatus(host_fips=True, rest_tls='fips', flight_system_openssl=None, pyarrow_version=None, pyarrow_error=None)
```

See [REST on a FIPS cluster](../user-guide/fips-rest.md) for a walkthrough in a UBI Python pod.

## REST and Flight SQL

Flight SQL needs pyarrow from the Red Hat AI Python index, and that pyarrow only runs in RHOAI workbench or AIPCC images with Python 3.12. Install and run the SDK in a workbench, or in a pod started from a workbench image. A plain UBI Python pod isn't enough, for two reasons:

- PyPI's pyarrow compiles its own OpenSSL into its libraries and never uses the host's FIPS provider. It can't be fixed after installation.
- Red Hat pyarrow links system libraries that plain UBI images don't ship, such as `libre2`, `libsnappy` and, in some versions, `libthrift`. On `ubi9/python-312`, `import pyarrow` fails with an error like `ImportError: libthrift-0.24.0.so: cannot open shared object file`.

**Python 3.12 is required.** The Red Hat AI Python index publishes pyarrow only for CPython 3.12, for RHEL 9 on x86_64, aarch64, ppc64le and s390x. With any other Python version, such as the Python 3.11 workbench images of older RHOAI releases, pip finds no Red Hat pyarrow: the install fails, or, if PyPI is also configured, pip installs PyPI's pyarrow. Check with `python3 --version` before installing.

### Choose a workbench image

Workbench images are ready for a FIPS install: they ship the system libraries, and they point pip and uv at the Red Hat AI Python index instead of PyPI through `PIP_CONFIG_FILE=/opt/app-root/pip.conf` and `UV_CONFIG_FILE=/opt/app-root/uv.toml`. Any Python 3.12 workbench image works; the **minimal** image is the smallest.

- **RHOAI dashboard:** create a workbench with a Python 3.12 image, for example **Jupyter | Minimal | CPU | Python 3.12**, and open a terminal or notebook in it.
- **A pod:** start a pod from the minimal workbench image. Take its reference from the `s2i-minimal-notebook` imagestream in `redhat-ods-applications`, whose tag annotated `opendatahub.io/workbench-image-recommended: "true"` matches the installed RHOAI version; registry tags don't exist for every RHOAI release, such as early-access builds. OpenShift clusters can pull from `registry.redhat.io` with their global pull secret. The image's entrypoint starts JupyterLab, so give the pod its own command, such as `sleep infinity`. `oc get imagestream -n redhat-ods-applications` lists all workbench images shipped with your RHOAI version.
- **Your own image:** build `FROM` a workbench or AIPCC Python 3.12 image so the system libraries and index configuration carry over, and don't override `PIP_INDEX_URL` or `PIP_CONFIG_FILE`.

### Install

In a workbench image, install as usual. pip and uv already use the Red Hat AI Python index:

```console
pip install "data-connect-hub[flight]>=0.3.0"
```

The index URL depends on the RHOAI version; `pip config list` shows it. Don't add PyPI as a second index. pip picks the highest version from any configured index, so PyPI's pyarrow can replace the Red Hat build. These all bring it back:

- `--extra-index-url https://pypi.org/simple`, or `PIP_INDEX_URL` or `PIP_EXTRA_INDEX_URL` pointing at PyPI
- `uv pip install --index https://pypi.org/simple`, or a uv project index pointing at PyPI
- `pip install -U pyarrow`, or a pyarrow version pin, while PyPI is configured
- Requirements or lock files generated against PyPI

A REST and Flight SQL setup reports:

```text
FipsStatus(host_fips=True, rest_tls='fips', flight_system_openssl=True, pyarrow_version='25.0.1', pyarrow_error=None)
```

See [REST and Flight SQL on FIPS-enabled OpenShift AI clusters](../user-guide/fips-rest-flight.md) for a walkthrough in a workbench pod.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `host_fips=False` | The node isn't in FIPS mode | Run on a FIPS-enabled cluster |
| `rest_tls='not-fips'` | Python carries its own OpenSSL, as uv-managed, python.org and conda builds do, or the FIPS provider isn't enforced | Use the image's own `python3` |
| `flight_system_openssl=False` | pyarrow came from PyPI | Reinstall it from the Red Hat index with `pip install --force-reinstall --no-deps pyarrow` in a workbench image, without PyPI configured, or start from a fresh workbench |
| `pyarrow_error` names a missing library, such as `libthrift` | Red Hat pyarrow outside an RHOAI or AIPCC image | Use a workbench image |
| `DCHConfigError: pyarrow is installed but failed to load` | Same as above | Same as above |
| `No matching distribution found for pyarrow` | Python isn't 3.12, so the Red Hat index has no pyarrow for it | Use a Python 3.12 workbench image |
| `ResolutionImpossible` mentioning `adbc-driver-flightsql` | An SDK version earlier than 0.3.0 | Install SDK 0.3.0 or later |

## Limits

- `insecure=True` disables certificate verification, not TLS encryption or the FIPS module. Use `ca_cert` to trust a private CA instead; it changes which CAs are trusted, not which module performs TLS.
- The SDK isn't a validated cryptographic module itself. The validated module is the host's OpenSSL FIPS provider; see Red Hat's RHEL FIPS documentation for its CMVP certificate.
- `fips_status()` and `require_fips` check the process they run in. They don't cover the Data Connect Hub services or the TLS configuration of the server you connect to.
