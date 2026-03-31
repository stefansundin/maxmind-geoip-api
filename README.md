This is a tiny MaxMind GeoIP API, written in rust for minimal resource usage. The docker image comes in at less than 5 MB uncompressed. This makes it convenient to run as a sidecar container.

You simply need to configure `MAXMIND_DB_URL` with a URL for your database, and `DATA_DIR` to define where to store the database, and run the program.

The program can automatically decompress archives of the formats `.zip`, `.tar`, `.gz`, `.bz2`, `.xz`, and `.zst`. It will check if there's a new database update every 24 hours. Update checks use the `ETag` header from the previous download to avoid downloading the file again if there isn't a new version available.

The database is written to disk and opened using mmap, so the program doesn't even need to read the full database into memory.

See the [examples](examples) directory to get started.


## API

### `GET /{ip}`

Look up a single IP address.

```shell
curl http://localhost:3000/1.2.3.4
```

Returns the GeoIP data as JSON with a `200` status, or `404` if the IP is not found in the database.

### `POST /lookup`

Look up multiple IP addresses in a single request (up to 1000).

```shell
curl -X POST http://localhost:3000/lookup \
  -H "Content-Type: application/json" \
  -d '["127.0.0.1", "8.8.8.8", "2607:F8B0:400A:801::200E"]'
```

Returns a JSON object mapping each IP to its GeoIP data, or `null` if not found:

```json
{
  "127.0.0.1": null,
  "2607:f8b0:400a:801::200e": { "continent": { ... }, "country": { ... }, ... },
  "8.8.8.8": { "continent": { ... }, "country": { ... }, ... }
}
```

Note that IPv6 addresses will be normalized to lowercase, as shown in the example above.

### `GET /metadata`

Returns the MaxMind database metadata.

```shell
curl http://localhost:3000/metadata
```


## Docker image

The docker image is available on [Docker Hub](https://hub.docker.com/r/stefansundin/maxmind-geoip-api) and [Amazon ECR](https://gallery.ecr.aws/stefansundin/maxmind-geoip-api).

```
stefansundin/maxmind-geoip-api:v1
```

```
ecr-public.aws.com/stefansundin/maxmind-geoip-api:v1
```


## Configuration

You can set these environment variables:

| Key          | Description | Default in docker | Default in binary | Required? |
|--------------|-------------|-------------------|-------------------|-----------|
| `DATA_DIR`                         | All the program state is stored in this directory, including the downloaded database file. | `/data` | No default | **Required** |
| `HOST`                             | The address that the program listens to. | `::` | `::` | **Required** |
| `PORT`                             | The port that the program listens to. | `80` | `3000` | **Required** |
| `MAXMIND_DB_URL`                   | The URL to download the MaxMind database file from. Most users should configure this to keep their database up to date. It is possible to run the program with a static database by placing a database file at `$DATA_DIR/database.mmdb` but it will not be updated automatically. | Not set | Not set | Optional |
| `CORS_ALLOWED_ORIGINS`             | Allowed origins for CORS. Useful if you want to call the service from JavaScript in web browsers. | No default | No default | Optional |
| `BATCH_LIMIT`                      | Limit of how many IPs can be looked up in a single `/lookup` batch request. | 1000 | 1000 | Optional |
| `CA_BUNDLE`                        | Path to a CA bundle to load. Useful if a self-signed certificate is used on the server hosting `MAXMIND_DB_URL`. | No default | No default | Optional |
| `DANGER_ACCEPT_INVALID_CERTS`      | Set to `true` to accept invalid certificates when downloading `MAXMIND_DB_URL`. | No default | No default | Optional |


## SIGHUP

If you want to force a database update check then send the program a SIGHUP signal:

```shell
docker ps
docker kill --signal=HUP container_id

# if you are using docker compose:
docker compose kill --signal=HUP geoip
```


## Development

```shell
export RUST_LOG=maxmind_geoip_api=debug
export DATA_DIR=$PWD/data
export MAXMIND_DB_URL=https://example.com/GeoLite2-City.mmdb
cargo run
```
