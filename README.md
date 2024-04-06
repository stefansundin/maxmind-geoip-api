This is a tiny MaxMind GeoIP API, written in rust for minimal resource usage. The docker image comes in at less than 5 MB uncompressed. This makes it convenient to run it as a sidecar container.

You simply need to configure `MAXMIND_DB_URL` with a URL that has your database and run the program. Then query the API by putting the desired IP address in the path, e.g. http://localhost:3000/1.2.3.4. Get the database metadata from http://localhost:3000/metadata.

The program can automatically decompress archives of the formats `.zip`, `.tar`, `.gz`, `.bz2`, and `.xz`. It will check if there's a new database update 24 hours. Update checks use the `ETag` header from the previous download to avoid downloading the file again if there isn't a new version available.

See the [examples](examples) directory to get started.


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
