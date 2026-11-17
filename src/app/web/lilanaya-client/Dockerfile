# Build context: this directory.
# nginx serves the static files and proxies /api to the backend. No build step.
FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY public /usr/share/nginx/html
EXPOSE 80
