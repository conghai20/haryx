# haryx-platform: HaryX web (this app) as static files behind nginx, built by deploy/compose.yaml.
# Build context is the repository root: the app reads apps/windows/src ("@win") and packages/.
#   docker buildx build --platform linux/amd64 -f apps/platform/Dockerfile \
#     -t wecoap/haryx:platform-v<version> -t wecoap/haryx:platform-vlatest --push .
FROM node:24-alpine AS build
WORKDIR /src/apps/platform
COPY apps/platform/package.json apps/platform/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY packages /src/packages
COPY apps/windows/src /src/apps/windows/src
COPY apps/platform ./
# A release always points at sso.wecoap.com, realm wecoap, client haryx-platform (src/lib/config.ts).
ENV VITE_SSO_URL= VITE_SSO_REALM= VITE_SSO_CLIENT=
RUN npm run check && npx vitest run && npm run build

# Unprivileged nginx (listens on 8080, runs as nginx, not root). It sets the page's security
# headers itself, so the host's reverse proxy only forwards /platform/.
FROM nginxinc/nginx-unprivileged:1.29-alpine
COPY apps/platform/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/apps/platform/dist /usr/share/nginx/html/platform
EXPOSE 8080
