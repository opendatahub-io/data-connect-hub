# Connector Reference

<style>
html,
body {
  max-width: 100%;
  overflow-x: hidden;
}

body {
  margin: 2rem auto;
  padding: 0 1rem;
  font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  line-height: 1.5;
}

table {
  width: 100%;
  max-width: 100%;
  border-collapse: collapse;
}

th,
td {
  min-width: 0;
  padding: 0.75rem;
  vertical-align: top;
  overflow-wrap: anywhere;
  word-break: break-word;
}

code {
  white-space: normal;
  overflow-wrap: anywhere;
}

@media (min-width: 1101px) {
  table,
  th,
  td {
    border: 1px solid #d0d7de;
  }

  thead th {
    background: #f6f8fa;
  }
}

@media (max-width: 1100px) {
  table,
  thead,
  tbody,
  tr,
  th,
  td {
    display: block;
    width: auto;
  }

  thead {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  tbody tr {
    margin: 1rem 0;
    padding: 0.75rem;
    border: 1px solid #d0d7de;
    border-radius: 0.4rem;
  }

  tbody th[scope="row"] {
    padding: 0 0 0.75rem;
    font-size: 1.1rem;
    text-align: left;
  }

  tbody td {
    display: grid;
    grid-template-columns: minmax(10rem, 32%) minmax(0, 1fr);
    gap: 0.75rem;
    padding: 0.6rem 0;
    border-top: 1px solid #d8dee4;
  }

  tbody td::before {
    font-weight: 600;
  }

  tbody td:nth-of-type(1)::before {
    content: "Unencrypted network transport";
  }

  tbody td:nth-of-type(2)::before {
    content: "TLS-encrypted network transport";
  }

  tbody td:nth-of-type(3)::before {
    content: "Supported ingestion";
  }

  tbody td:nth-of-type(4)::before {
    content: "Input format";
  }
}
</style>

<table style="width: 100%;">
  <colgroup>
    <col style="width: 16%;">
    <col style="width: 24%;">
    <col style="width: 24%;">
    <col style="width: 14%;">
    <col style="width: 22%;">
  </colgroup>
  <thead>
    <tr>
      <th scope="col">Connector</th>
      <th scope="col">Unencrypted network transport</th>
      <th scope="col">TLS-encrypted network transport</th>
      <th scope="col">Supported ingestion</th>
      <th scope="col">Input format</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <th scope="row"><a href="https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/postgres.yaml"><code>postgres</code></a></th>
      <td>
        <ul>
          <li>User provides a non-TLS PostgreSQL URI</li>
          <li>Example: <code>postgresql://host/db?sslmode=disable</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>User provides a TLS PostgreSQL URI</li>
          <li>User sets a custom CA via <code>CA_CERT</code></li>
          <li>Example: <code>postgresql://host/db?sslmode=verify-ca</code></li>
        </ul>
      </td>
      <td><p>Tabular</p></td>
      <td>
        <p>Read-only SQL query</p>
        <ul><li>Example: <code>SELECT id, name FROM users LIMIT 10</code></li></ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><code>sqlite</code></th>
      <td><strong>N/A</strong> — local file access</td>
      <td><strong>N/A</strong> — local file access</td>
      <td><p>Tabular</p></td>
      <td>
        <p>Read-only SQL query</p>
        <ul><li>Example: <code>SELECT name FROM users LIMIT 10</code></li></ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><a href="https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/elasticsearch.yaml"><code>elasticsearch</code></a></th>
      <td>
        <ul>
          <li>User provides an <code>http://</code> endpoint</li>
          <li>Example: <code>http://host:9200</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>User provides an <code>https://</code> endpoint</li>
          <li>User sets a custom CA via <code>ES_CA_CERT</code></li>
          <li>Example: <code>https://host:9200</code></li>
        </ul>
      </td>
      <td><p>Tabular</p></td>
      <td>
        <p>Elasticsearch Query DSL JSON</p>
        <ul>
          <li>Index in request or connection properties</li>
          <li>Example: <code>{"index":"products","query":{"match_all":{}}}</code></li>
        </ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><a href="https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/milvus.yaml"><code>milvus</code></a></th>
      <td>
        <ul>
          <li>User provides <code>MILVUS_URI</code> with <code>http://</code></li>
          <li>Example: <code>http://host:19530</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>User provides <code>MILVUS_URI</code> with <code>https://</code></li>
          <li>User sets a custom CA via <code>MILVUS_CA_CERT</code> (optional PEM CA certificate)</li>
          <li>Example: <code>https://host:8080</code> (TLS REST port used by this repository's installation script)</li>
        </ul>
      </td>
      <td><p>Tabular</p></td>
      <td>
        <p>Milvus REST API JSON</p>
        <ul>
          <li>Query</li>
          <li>Vector search</li>
          <li>Get by ID</li>
          <li>Example: <code>{"collectionName":"products","filter":"price &gt; 50"}</code></li>
        </ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><a href="https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/neo4j.yaml"><code>neo4j</code></a></th>
      <td>
        <ul>
          <li>User provides a <code>neo4j://</code> URI</li>
          <li>Example: <code>neo4j://host:7687</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>User provides a <code>neo4j+s://</code> URI</li>
          <li>User sets a custom CA via <code>NEO4J_CA_CERT</code></li>
          <li>Example: <code>neo4j+s://host:7687</code></li>
        </ul>
      </td>
      <td><p>Tabular</p></td>
      <td>
        <p>Cypher query</p>
        <ul><li>Example: <code>MATCH (n:Person) RETURN n.name LIMIT 10</code></li></ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><code>s3</code></th>
      <td>
        <ul>
          <li>User sets <code>AWS_S3_ENDPOINT</code> to an <code>http://</code> endpoint</li>
          <li>Example: <code>http://host:9000</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>User provides an HTTPS S3 endpoint</li>
          <li>Example: <code>https://s3.amazonaws.com</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>Binary</li>
          <li>Tabular</li>
        </ul>
      </td>
      <td>
        <p>Object path</p>
        <ul>
          <li>Parquet, CSV, or JSON Lines</li>
          <li>Format from extension or <code>format</code> property</li>
          <li>Example: <code>data/events.parquet</code></li>
        </ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><a href="https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/uri.yaml"><code>uri</code></a></th>
      <td>
        <ul>
          <li>User provides a base URI with <code>http://</code></li>
          <li>Example: <code>http://host/</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>User provides a base URI with <code>https://</code></li>
          <li>User sets a custom CA via <code>CA_CERT</code></li>
          <li>Example: <code>https://host/</code></li>
        </ul>
      </td>
      <td>
        <ul>
          <li>Binary</li>
          <li>Tabular</li>
        </ul>
      </td>
      <td>
        <p>GET request JSON</p>
        <ul>
          <li><code>path</code> required</li>
          <li><code>data_path</code> optional</li>
          <li>Example: <code>{"path":"/api/data"}</code></li>
        </ul>
      </td>
    </tr>
  </tbody>
</table>
