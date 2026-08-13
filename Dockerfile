FROM node:24-alpine

WORKDIR /app

COPY package.json package-lock.json* ./
RUN npm install --omit=dev

COPY . .

ENV NODE_ENV=production
ENV PORT=8000
ENV HOST=::
ENV DB_PATH=/app/data/calendar.db

RUN mkdir -p /app/data

EXPOSE 8000
CMD ["node", "server.ts"]
