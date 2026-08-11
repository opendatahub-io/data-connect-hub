"""Arrow Flight SQL client for tabular data queries."""

from __future__ import annotations

import contextlib
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    import pandas as pd

import adbc_driver_flightsql.dbapi as flight_dbapi
import pyarrow as pa

from ._auth import ADBC_HEADER_PREFIX, build_flight_headers
from .exceptions import DCHConnectionError, DCHQueryError


class FlightSQLClient:
    """Thin wrapper around ADBC Flight SQL for Data Connect Hub queries.

    Parameters
    ----------
    flight_url : str
        gRPC endpoint, e.g. ``grpc://host:50051``.
    token : str
        Raw Bearer token value.
    tenant_id : str
        Tenant identifier.
    """

    def __init__(self, flight_url: str, token: str = "", tenant_id: str = "") -> None:
        self._flight_url = flight_url
        self._token = token
        self._tenant_id = tenant_id
        self._base_kwargs = build_flight_headers(token=token, tenant_id=tenant_id)

    def _connect(self, connection_id: str) -> flight_dbapi.Connection:
        db_kwargs = {
            **self._base_kwargs,
            f"{ADBC_HEADER_PREFIX}x-data-connection-id": connection_id,
        }
        try:
            return flight_dbapi.connect(self._flight_url, db_kwargs=db_kwargs)
        except (flight_dbapi.InterfaceError, flight_dbapi.OperationalError) as exc:
            raise DCHConnectionError(str(exc)) from exc

    def read(self, sql: str, connection_id: str) -> pa.Table:
        """Execute *sql* and return the full result as a PyArrow Table."""
        conn = self._connect(connection_id)
        try:
            cursor = conn.cursor()
            try:
                cursor.execute(sql)
                return cursor.fetch_arrow_table()
            except flight_dbapi.Error as exc:
                raise DCHQueryError(str(exc)) from exc
            finally:
                with contextlib.suppress(Exception):
                    cursor.close()
        finally:
            conn.close()

    def read_pandas(self, sql: str, connection_id: str) -> pd.DataFrame:
        """Execute *sql* and return the result as a pandas DataFrame."""
        return self.read(sql, connection_id).to_pandas()

    def server_info(self) -> dict[str, Any]:
        """Return Flight SQL server metadata."""
        try:
            conn = flight_dbapi.connect(self._flight_url, db_kwargs=self._base_kwargs)
        except (flight_dbapi.InterfaceError, flight_dbapi.OperationalError) as exc:
            raise DCHConnectionError(str(exc)) from exc
        try:
            return {str(k): v for k, v in conn.adbc_get_info().items()}
        except (flight_dbapi.InterfaceError, flight_dbapi.OperationalError) as exc:
            raise DCHConnectionError(str(exc)) from exc
        finally:
            conn.close()

    def close(self) -> None:
        """No-op — connections are opened and closed per call."""
