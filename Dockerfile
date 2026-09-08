# Zweistufiger Bau: statisch gegen musl übersetzen, dann in ein Image ohne
# Betriebssystem legen. Das Ergebnis enthält genau eine Datei — die Binärdatei —
# und damit keine Shell, keinen Paketmanager und nichts, was zu aktualisieren
# wäre.
#
# Warum das hier überhaupt aufgeht: reqwest ist auf rustls mit gebündeltem
# Wurzelzertifikatsspeicher (webpki-roots) eingestellt, siehe Cargo.toml. Ein
# Image mit OpenSSL oder einem CA-Bundle aus dem System wird deshalb nicht
# gebraucht — sonst scheiterte jeder HTTPS-Abruf der Landesdienste.

FROM rust:1.97-alpine AS builder

# musl-dev und gcc werden gebraucht: zstd-sys und ring übersetzen C-Quellen mit.
RUN apk add --no-cache musl-dev gcc

WORKDIR /build

# Erst nur die Abhängigkeiten übersetzen, gegen eine leere Quelltextattrappe.
# Diese Schicht überlebt jede Änderung am eigenen Code — ohne sie würde bei
# jedem Bau der gesamte Abhängigkeitsbaum neu übersetzt, und mit lto=true und
# codegen-units=1 dauert das einige Minuten.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && : > src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src

COPY src ./src
# Die Attrappe hat den Fingerabdruck der Ziele bereits gesetzt; touch stellt
# sicher, dass Cargo den echten Quelltext auch dann neu übersetzt, wenn die
# Zeitstempel aus dem Baukontext älter sein sollten.
RUN touch src/main.rs src/lib.rs \
 && cargo build --release --locked \
 && strip target/release/alkis-proxy

# Verzeichnis für die redb-Cache-Datei mit korrektem Besitzer vorbereiten:
# Das distroless-Image hat keine Shell, in der sich das nachträglich per
# `mkdir`/`chown` erledigen ließe. Ein Docker-Volume, das beim ersten Start
# hierauf gemountet wird, übernimmt Inhalt und Rechte dieses Verzeichnisses.
RUN mkdir -p /empty-data && chown 65532:65532 /empty-data

# `static` genügt, weil die Binärdatei statisch gegen musl gebunden ist.
# `nonroot` setzt UID 65532 — der Dienst braucht keine Rechte im Container.
FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=builder /build/target/release/alkis-proxy /usr/local/bin/alkis-proxy
COPY --from=builder --chown=65532:65532 /empty-data /data

# Innerhalb des Containers immer 8080; nach außen bildet Compose das ab.
ENV ALKIS_BIND=0.0.0.0:8080 \
    ALKIS_CACHE_PATH=/data/cache.redb \
    RUST_LOG=alkis_proxy=info,tower_http=warn
EXPOSE 8080

USER nonroot:nonroot

# Bewusst keine HEALTHCHECK-Anweisung: Das Image hat weder Shell noch curl,
# also gäbe es nichts, womit sie sich ausführen ließe. Der Dienst beantwortet
# GET /health — diesen Endpunkt prüft man von außen, aus dem Reverse Proxy
# oder dem Orchestrierer.
ENTRYPOINT ["/usr/local/bin/alkis-proxy"]
