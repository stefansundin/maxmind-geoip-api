This is a tiny MaxMind GeoIP API, written in rust for minimal resource usage. The docker image comes in at less than 5 MB uncompressed. This makes it convenient to run as a sidecar container.

You simply need to configure `MAXMIND_DB_URL` with a URL that has your database and run the program. Then query the API by putting the desired IP address in the path, e.g. http://localhost:3000/1.2.3.4. Get the database metadata from http://localhost:3000/metadata.

The program can automatically decompress archives of the formats `.zip`, `.tar`, `.gz`, `.bz2`, `.xz`, and `.zst`. It will check if there's a new database update every 24 hours. Update checks use the `ETag` header from the previous download to avoid downloading the file again if there isn't a new version available.

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
  -d '["1.2.3.4", "8.8.8.8"]'
```

Returns a JSON object mapping each IP to its GeoIP data, or `null` if not found:

```json
{
  "1.2.3.4": { "city": { ... }, "country": { ... }, ... },
  "8.8.8.8": { "city": { ... }, "country": { ... }, ... }
}
```

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
public.ecr.aws/stefansundin/maxmind-geoip-api:v1
```


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
