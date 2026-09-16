# Connector Reference

<table style="table-layout: fixed; width: 100%;">
  <thead>
    <tr>
      <th scope="col">Connector</th>
      <th scope="col">Unencrypted network transport</th>
      <th scope="col">TLS-encrypted network transport</th>
      <th scope="col">Supported ingestion</th>
      <th scope="col">Credential fields</th>
      <th scope="col">Input format</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <th scope="row"><code>postgres</code></th>
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
        <ul>
          <li><code>URI</code> — required</li>
          <li><code>CA_CERT</code> — optional PEM CA</li>
        </ul>
      </td>
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
        <ul>
          <li><code>URI</code> — required</li>
          <li>Example: <code>sqlite://data/app.db</code></li>
        </ul>
      </td>
      <td>
        <p>Read-only SQL query</p>
        <ul><li>Example: <code>SELECT name FROM users LIMIT 10</code></li></ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><code>elasticsearch</code></th>
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
        <ul>
          <li><code>ES_URI</code> — required</li>
          <li><code>ES_API_KEY</code> — optional</li>
          <li><code>ES_USERNAME</code> + <code>ES_PASSWORD</code> — optional</li>
          <li><code>ES_CA_CERT</code> — optional PEM CA</li>
        </ul>
      </td>
      <td>
        <p>Elasticsearch Query DSL JSON</p>
        <ul>
          <li>Index in request or connection properties</li>
          <li>Example: <code>{"index":"products","query":{"match_all":{}}}</code></li>
        </ul>
      </td>
    </tr>
    <tr>
      <th scope="row"><code>milvus</code></th>
      <td>
        <ul>
          <li>User provides <code>MILVUS_URI</code> with <code>http://</code></li>
          <li>Example: <code>http://host:19530</code></li>
        </ul>
      </td>
      <td></td>
      <td><p>Tabular</p></td>
      <td>
        <ul>
          <li><code>MILVUS_URI</code> — required</li>
          <li><code>MILVUS_TOKEN</code> — optional</li>
          <li><code>MILVUS_DATABASE</code> — optional</li>
        </ul>
      </td>
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
      <th scope="row"><code>neo4j</code></th>
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
        <ul>
          <li><code>NEO4J_URI</code> — required</li>
          <li><code>NEO4J_USERNAME</code> — optional; defaults to <code>neo4j</code></li>
          <li><code>NEO4J_PASSWORD</code> — required</li>
          <li><code>NEO4J_DATABASE</code> — optional; defaults to <code>neo4j</code></li>
          <li><code>NEO4J_CA_CERT</code> — optional</li>
        </ul>
      </td>
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
        <ul>
          <li><code>AWS_S3_BUCKET</code> — required</li>
          <li><code>AWS_ACCESS_KEY_ID</code> — required</li>
          <li><code>AWS_SECRET_ACCESS_KEY</code> — required</li>
          <li><code>AWS_DEFAULT_REGION</code> — optional</li>
          <li><code>AWS_S3_ENDPOINT</code> — optional</li>
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
      <th scope="row"><code>uri</code></th>
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
        <ul>
          <li><code>URI</code> — required</li>
          <li><code>TOKEN</code> — optional bearer token</li>
          <li><code>USERNAME</code> + <code>PASSWORD</code> — optional Basic Auth</li>
          <li><code>CA_CERT</code> — optional PEM CA</li>
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
