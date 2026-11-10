#!/usr/bin/env python3
"""Exercise the built server, scheduler, SQLite and real HTTP sockets without live services.

Run: python3 scripts/test_e2e.py [target/debug/vesper]
All data is disposable. Telegram transport is covered separately by engine tests;
this suite does not send notifications or change a homelab.
"""
import datetime
import http.server
import json
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


def await_value(read, predicate, description, timeout=12):
    deadline = time.monotonic() + timeout
    value = None
    while time.monotonic() < deadline:
        value = read()
        if predicate(value):
            return value
        time.sleep(0.1)
    raise AssertionError(f"Timed out: {description}; last value: {value}")


class Endpoint(http.server.BaseHTTPRequestHandler):
    status = 200

    def do_GET(self):
        self.send_response(type(self).status)
        self.end_headers()
        self.wfile.write(b"fixture ready")

    def log_message(self, *_):
        pass


def main():
    binary = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/vesper").resolve()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as discovery:
        # UDP connect selects an interface; it sends no packet.
        discovery.connect(("192.0.2.1", 9))
        private_ip = os.environ.get("VESPER_TEST_LAN_IPV4", discovery.getsockname()[0])
    endpoint = http.server.ThreadingHTTPServer(("0.0.0.0", 0), Endpoint)
    worker = threading.Thread(target=endpoint.serve_forever, daemon=True)
    worker.start()
    process = None
    origin = "https://vesper.example.test"
    token = "disposable-e2e-push-token-not-a-production-secret"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    with tempfile.TemporaryDirectory(prefix="vesper-e2e-") as temporary:
        root = pathlib.Path(temporary)
        for child in ["config", "data", "secrets", "web"]:
            (root / child).mkdir()
        (root / "secrets/push").write_text(token)
        (root / "web/index.html").write_text("<!doctype html><title>Vesper test bundle</title>")
        # JSON is a YAML subset; no fixture-only parser or secret shortcut is used.
        configuration = {
            "defaults": {"interval_seconds": 60, "timeout_seconds": 2, "stale_after_seconds": 180},
            "network": {"allowed_cidrs": [f"{private_ip}/32"]},
            "machines": {"home": {"name": "Home", "site": "Lab", "services": {
                "web": {"name": "Web availability", "checks": {"http": {
                    "kind": "http", "url": f"http://{private_ip}:{endpoint.server_port}/", "body_contains": "ready"
                }}},
                "backup": {"name": "Backups", "checks": {"push": {
                    "kind": "push", "token": {"file": "push"}, "expected_interval_seconds": 2, "grace_seconds": 1
                }}}
            }}}
        }
        config_file = root / "config/_index.yaml"
        original = json.dumps(configuration)
        config_file.write_text(original)
        env = os.environ.copy()
        env.update(VESPER_CONFIG_DIR=str(root / "config"), VESPER_DATA_DIR=str(root / "data"),
                   VESPER_SECRETS_DIR=str(root / "secrets"), VESPER_STATIC_DIR=str(root / "web"),
                   VESPER_PUBLIC_URL=origin, VESPER_TRUSTED_PROXIES="127.0.0.1/32", VESPER_DEV_MODE="false")
        # Exercise exported block YAML as an editable installation, not just JSON import.
        original = subprocess.run([str(binary), "export"], env=env, check=True, capture_output=True, text=True).stdout
        config_file.write_text(original)
        validate = subprocess.run([str(binary), "validate"], env=env, check=True, capture_output=True, text=True)
        print(validate.stdout.strip())
        with socket.socket() as port_socket:
            port_socket.bind(("127.0.0.1", 0))
            port = port_socket.getsockname()[1]
        env["VESPER_LISTEN"] = f"127.0.0.1:{port}"
        base = f"http://127.0.0.1:{port}"
        csrf = None
        log = (root / "server.log").open("w+")

        def call(path, data=None, method=None, heartbeat=False, credential=token, expected=200):
            headers = {}
            if data is not None:
                headers["Content-Type"] = "application/json"
                if heartbeat:
                    headers["Authorization"] = f"Bearer {credential}"
                else:
                    headers.update(Origin=origin, **{"X-Vesper-CSRF": csrf or ""})
            request = urllib.request.Request(base + path, data=json.dumps(data).encode() if data is not None else None,
                                             headers=headers, method=method)
            try:
                response = opener.open(request, timeout=8)
            except urllib.error.HTTPError as error:
                response = error
            payload = response.read()
            assert response.status == expected, (path, response.status, payload.decode())
            return json.loads(payload)

        def start(read_only=False):
            nonlocal process, csrf
            env["VESPER_CONFIG_READ_ONLY"] = str(read_only).lower()
            process = subprocess.Popen([str(binary), "serve"], env=env, stdout=log, stderr=log)
            for _ in range(100):
                if process.poll() is not None:
                    log.flush(); log.seek(0)
                    raise AssertionError(log.read())
                try:
                    call("/healthz")
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.05)
            else:
                raise AssertionError("Server did not become ready")
            csrf = call("/api/csrf")["token"]

        def stop():
            nonlocal process
            if process:
                process.terminate()
                process.wait(timeout=10)
                process = None

        def snapshot():
            return call("/api/snapshot")

        def service(name):
            return next(s for s in snapshot()["machines"][0]["services"] if s["id"] == f"home/{name}")

        def check_again(status, expected_health):
            Endpoint.status = status
            previous = service("web")["checks"][0]["last_checked"]
            call("/api/services/home/web/recheck", {})
            result = await_value(lambda: service("web"), lambda s: s["checks"][0]["last_checked"] != previous,
                                 "fresh active check")
            assert result["health"] == expected_health, result
            print(f"HTTP {status} → {expected_health}")

        try:
            start()
            await_value(lambda: service("web")["health"], lambda h: h == "healthy", "initial HTTP success")
            check_again(503, "suspect")
            check_again(503, "suspect")
            check_again(503, "down")
            incident = next(i for i in snapshot()["incidents"] if i["service_id"] == "home/web")
            call(f"/api/incidents/{incident['id']}/acknowledge", {"acknowledged": True})
            until = (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=1)).isoformat()
            call("/api/services/home/web/silence", {"until": until})
            assert service("web")["health"] == "down"
            check_again(200, "recovering")
            check_again(503, "down")
            check_again(200, "recovering")
            check_again(200, "healthy")
            web_incidents = [i for i in snapshot()["incidents"] if i["service_id"] == "home/web"]
            assert len(web_incidents) == 1 and web_incidents[0]["resolved_at"] and web_incidents[0]["acknowledged_at"]
            print("Flapping, acknowledgement and silence preserved one incident; two passes recovered")

            call("/api/heartbeat/home/backup/push", {"healthy": True}, heartbeat=True, credential="incorrect", expected=401)
            call("/api/heartbeat/home/backup/push", {"healthy": True}, heartbeat=True)
            assert service("backup")["health"] == "healthy"
            await_value(lambda: service("backup")["health"], lambda h: h == "down", "push missed deadline", timeout=7)
            samples = len(service("backup")["checks"][0]["history"])
            time.sleep(2)
            assert len(service("backup")["checks"][0]["history"]) == samples, "scheduler manufactured failed heartbeats"
            call("/api/heartbeat/home/backup/push", {"healthy": True}, heartbeat=True)
            assert service("backup")["health"] == "healthy"
            print("Push token isolation, missed deadline without synthetic heartbeats and immediate recovery passed")

            before = service("web")["checks"][0]["last_checked"]
            draft = call("/api/draft/test", {"checks": {"blocked": {"kind": "http", "url": "http://127.0.0.1:1/"}}})
            assert draft["results"]["blocked"]["healthy"] is False
            assert service("web")["checks"][0]["last_checked"] == before
            print("Draft network policy rejection left active monitoring unchanged")

            config_file.write_text("machines: [broken")
            await_value(snapshot, lambda s: s["config_error"] is not None, "invalid reload diagnostic", timeout=8)
            assert service("web")["name"] == "Web availability"
            config_file.write_text(original)
            await_value(snapshot, lambda s: s["config_error"] is None, "valid configuration restored", timeout=8)
            active = call("/api/config")
            updated_service = active["machines"]["home"]["services"]["web"]
            updated_service["name"] = "Web edited through API"
            call("/api/services/home/web", {"revision": "stale", "service": updated_service}, method="PUT", expected=409)
            assert config_file.read_text() == original
            call("/api/services/home/web", {"revision": active["revision"], "service": updated_service}, method="PUT")
            assert service("web")["name"] == "Web edited through API"
            print("Invalid reload retained inventory; stale revision rejected; valid source edit activated")

            incident_count = len(snapshot()["incidents"])
            stop()
            start(read_only=True)
            assert len(snapshot()["incidents"]) == incident_count, "restart manufactured an incident"
            assert service("web")["checks"][0]["history"], "history lost across restart"
            read_only = call("/api/config")
            assert read_only["writable"] is False
            call("/api/services/home/web", {"revision": read_only["revision"], "service": updated_service}, method="PUT", expected=403)
            allowed = call("/api/draft/test", {"checks": configuration["machines"]["home"]["services"]["web"]["checks"]})
            assert allowed["results"]["http"]["healthy"] is True
            exported = subprocess.run([str(binary), "export"], env=env, check=True, capture_output=True, text=True).stdout
            assert token not in exported and 'file: push' in exported
            print("Restart retained history/incidents; read-only rejected writes but allowed tests and secret-reference export")
            print("PASS: real server + scheduler + HTTP fixture + SQLite end-to-end checks")
        finally:
            stop()
            endpoint.shutdown()
            endpoint.server_close()
            log.close()


if __name__ == "__main__":
    main()
