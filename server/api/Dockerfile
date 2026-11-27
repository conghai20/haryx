# haryx-api, built by deploy/compose.yaml (`build: ../server/api`).
FROM golang:1.27-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /haryx-api ./cmd/haryx-api

# CA certificates (for the realm's JWKS over HTTPS) and a non-root user, nothing else.
FROM gcr.io/distroless/static-debian13:nonroot
COPY --from=build /haryx-api /haryx-api
EXPOSE 8080
ENTRYPOINT ["/haryx-api"]
