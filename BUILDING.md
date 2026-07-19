You can build the docker image by running:

```shell
# Simple build for your current architecture:
docker build --pull --progress plain -t maxmind-geoip-api .
```

To build a multi-arch docker image you can run:

```shell
# Use buildx to build multi-arch images:
docker buildx create --use --name multiarch --node multiarch0

# Beta release (may be limited to one architecture):
docker buildx build --pull --push --progress plain --platform linux/arm64 -t stefansundin/maxmind-geoip-api:beta .

# You probably want to change the tag name if you are not me:
docker buildx build --pull --push --progress plain --platform linux/arm64,linux/amd64,linux/riscv64 -t stefansundin/maxmind-geoip-api:v1.3.0 .

# If the new version is stable then update tags:
docker buildx imagetools create -t stefansundin/maxmind-geoip-api:latest stefansundin/maxmind-geoip-api:v1.3.0
docker buildx imagetools create -t stefansundin/maxmind-geoip-api:v1 stefansundin/maxmind-geoip-api:latest
docker buildx imagetools create -t public.ecr.aws/stefansundin/maxmind-geoip-api:latest stefansundin/maxmind-geoip-api:latest
docker buildx imagetools create -t public.ecr.aws/stefansundin/maxmind-geoip-api:v1.3.0 stefansundin/maxmind-geoip-api:latest
docker buildx imagetools create -t public.ecr.aws/stefansundin/maxmind-geoip-api:v1 stefansundin/maxmind-geoip-api:latest
```

If the build crashes then it is most likely because Docker ran out of memory. Increase the amount of RAM allocated to Docker and quit other programs during the build. You can also try passing `--build-arg CARGO_BUILD_JOBS=1` to docker. To build one architecture at a time, [use `--config` when creating the builder instance](https://gist.github.com/stefansundin/fa1c1dd7a60ebe2f8a2aa6d32631b119).

## Podman

Cross-compiling amd64 from arm64 seems to be crashing Docker recently. So I have built `v1.2.0` and later using podman.

```shell
# Make sure you have installed qemu-user-static and qemu-user-static-binfmt to build cross architecture.

# Create a personal access token on docker.com and use it as the password:
podman login docker.io

podman manifest rm docker.io/stefansundin/maxmind-geoip-api:beta
podman manifest create docker.io/stefansundin/maxmind-geoip-api:beta
podman build --pull --platform linux/arm64,linux/amd64,linux/riscv64 --manifest docker.io/stefansundin/maxmind-geoip-api:beta .
podman manifest push docker.io/stefansundin/maxmind-geoip-api:beta

skopeo copy --all docker://docker.io/stefansundin/maxmind-geoip-api:beta docker://docker.io/stefansundin/maxmind-geoip-api:v1.3.0
skopeo copy --all docker://docker.io/stefansundin/maxmind-geoip-api:beta docker://docker.io/stefansundin/maxmind-geoip-api:v1
skopeo copy --all docker://docker.io/stefansundin/maxmind-geoip-api:beta docker://docker.io/stefansundin/maxmind-geoip-api:latest

# Get a login to Public ECR:
aws ecr-public get-login-password --region us-east-1
podman login public.ecr.aws --username AWS

skopeo copy --all docker://docker.io/stefansundin/maxmind-geoip-api:latest docker://public.ecr.aws/stefansundin/maxmind-geoip-api:latest
skopeo copy --all docker://docker.io/stefansundin/maxmind-geoip-api:latest docker://public.ecr.aws/stefansundin/maxmind-geoip-api:v1.3.0
skopeo copy --all docker://docker.io/stefansundin/maxmind-geoip-api:latest docker://public.ecr.aws/stefansundin/maxmind-geoip-api:v1
```
