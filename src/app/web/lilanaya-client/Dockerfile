# Build context: this directory.
# nginx serves the static files and proxies /api to the backend. No build step.
FROM nginx:1.30-alpine
# Where the API is reached; the host sets it (compose: api:8080).
ENV API_HOSTPORT=api:8080
# Makes the image's start-up script put the container's DNS servers in NGINX_LOCAL_RESOLVERS.
ENV NGINX_ENTRYPOINT_LOCAL_RESOLVERS=1
COPY security-headers.conf /etc/nginx/security-headers.conf
COPY default.conf.template /etc/nginx/templates/default.conf.template
COPY public /usr/share/nginx/html
EXPOSE 80
