import os
import sys
import json
import asyncio
import logging
import secrets
import hashlib
import time
import ctypes
from ctypes import wintypes
from typing import Dict, Any, Optional, Set
from collections import deque
from urllib.parse import urlparse

# Pastikan konsol Windows mendukung karakter emoji & utf-8
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        # pyrefly: ignore [missing-attribute]
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

import aiohttp
from aiohttp import web  # type: ignore
from telethon import TelegramClient  # type: ignore
from telethon.sessions import StringSession  # type: ignore

from config import Config
from recognizer import get_recognizer
from core import Claimer, WaifuListener
from core.claimer import _build_name_candidates


# --- Helper Memori Windows (0 dependensi pihak ketiga) ---
class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def get_current_memory_mb() -> float:
    """Mengukur penggunaan memori Working Set proses Python saat ini dalam MB."""
    try:
        counters = PROCESS_MEMORY_COUNTERS()
        counters.cb = ctypes.sizeof(PROCESS_MEMORY_COUNTERS)
        fn = ctypes.windll.psapi.GetProcessMemoryInfo
        fn.argtypes = [ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD]
        ctypes.windll.kernel32.GetCurrentProcess.restype = ctypes.c_void_p
        handle = ctypes.windll.kernel32.GetCurrentProcess()
        if fn(handle, ctypes.byref(counters), counters.cb):
            return round(counters.WorkingSetSize / (1024 * 1024), 1)
    except Exception:
        pass
    return 0.0


# --- Logging ke Circular Buffer untuk SSE Web Stream ---
class WebLogHandler(logging.Handler):
    """
    Handler khusus untuk streaming log ke antarmuka web (Live Console).
    Secara otomatis menyaring log bising (polling status HTTP, internal keepalive Telethon)
    agar tampilan konsol bersih, informatif, dan hanya berfokus pada aktivitas bot.
    """
    def __init__(self, maxlen: int = 200):
        super().__init__()
        self.buffer = deque(maxlen=maxlen)
        self.subscribers: Set[asyncio.Queue] = set()

    def emit(self, record: logging.LogRecord):
        # 1. Filter log bising dari library eksternal
        # Abaikan log akses polling web (misal: GET /api/status setiap beberapa detik)
        if record.name.startswith("aiohttp.access"):
            return
        # Abaikan pesan internal Telethon (keepalive MTProto, perbedaan channel rutin) kecuali WARNING/ERROR
        if record.name.startswith("telethon") and record.levelno < logging.WARNING:
            return
        # Abaikan level DEBUG di live console
        if record.levelno < logging.INFO:
            return

        msg = record.getMessage()

        # Tentukan modul ringkas (antikarbit.claimer -> claimer, dll)
        mod = record.name.replace("antikarbit.", "") if "antikarbit" in record.name else record.name
        time_str = time.strftime("%H:%M:%S", time.localtime(record.created))

        level_map = {
            "INFO": "info",
            "WARNING": "warning",
            "ERROR": "error",
            "CRITICAL": "error",
        }
        level = level_map.get(record.levelname, "info")

        # Sorot pesan sukses / penting
        if any(w in msg for w in ("✅", "🎉", "Sukses", "BERHASIL", "now protected", "added to your")):
            level = "success"

        # Format ringkas & bersih: [HH:MM:SS] [modul] pesan
        clean_msg = f"[{time_str}] [{mod}] {msg}"

        entry = {"message": clean_msg, "level": level}
        self.buffer.append(entry)

        # Broadcast ke semua web clients yang sedang aktif
        for q in list(self.subscribers):
            try:
                q.put_nowait(entry)
            except asyncio.QueueFull:
                pass


# Setup loggers
web_log_handler = WebLogHandler(maxlen=200)

clean_console_formatter = logging.Formatter(
    fmt="[%(asctime)s] [%(levelname)s] %(name)s: %(message)s",
    datefmt="%H:%M:%S",
)
web_log_handler.setFormatter(clean_console_formatter)

console_stream = logging.StreamHandler(sys.stdout)
console_stream.setFormatter(clean_console_formatter)

logging.basicConfig(
    level=logging.INFO,
    handlers=[
        console_stream,
        web_log_handler,
    ],
)

# Redam kebisingan log dari library eksternal
logging.getLogger("aiohttp.access").setLevel(logging.WARNING)
logging.getLogger("telethon").setLevel(logging.WARNING)

logger = logging.getLogger("antikarbit.app")


# --- Autentikasi & Keamanan Dashboard ---
# Token session yang valid disimpan di set ini (in-memory)
_valid_sessions: Set[str] = set()
# Rate limiter login: ip -> [timestamp, ...]
_login_attempts: Dict[str, list] = {}
_BRUTE_FORCE_WINDOW_SEC = 60   # Jendela waktu pengecekan login
_BRUTE_FORCE_MAX_ATTEMPTS = 5  # Maks percobaan gagal per jendela

# Global API rate limiter (anti-flood / DoS)
_api_request_log: Dict[str, list] = {}
_API_RATE_WINDOW_SEC = 10     # Jendela waktu 10 detik
_API_RATE_MAX_REQUESTS = 60    # Maks 60 request per 10 detik per IP
_UPLOAD_MAX_BYTES = 8 * 1024 * 1024  # Maksimum upload 8 MB


def get_client_ip(request) -> str:
    """Mendeteksi IP asli client dengan dukungan reverse proxy (Railway, Cloudflare, Nginx)."""
    cf_ip = request.headers.get("CF-Connecting-IP")
    if cf_ip:
        return cf_ip.strip()
    x_forwarded = request.headers.get("X-Forwarded-For")
    if x_forwarded:
        return x_forwarded.split(",")[0].strip()
    x_real = request.headers.get("X-Real-IP")
    if x_real:
        return x_real.strip()
    return request.remote or "127.0.0.1"


def _generate_session_token() -> str:
    return secrets.token_urlsafe(32)


def _hash_password(password: str) -> str:
    return hashlib.sha256(password.encode()).hexdigest()


def _is_brute_forced(ip: str) -> bool:
    """Cek apakah IP ini sudah melebihi batas percobaan login gagal."""
    now = time.time()
    attempts = _login_attempts.get(ip, [])
    # Hanya hitung percobaan dalam window terakhir
    attempts = [t for t in attempts if now - t < _BRUTE_FORCE_WINDOW_SEC]
    _login_attempts[ip] = attempts
    return len(attempts) >= _BRUTE_FORCE_MAX_ATTEMPTS


def _record_failed_attempt(ip: str):
    now = time.time()
    if ip not in _login_attempts:
        _login_attempts[ip] = []
    _login_attempts[ip].append(now)


def _clear_attempts(ip: str):
    _login_attempts.pop(ip, None)


def _is_api_rate_limited(ip: str) -> bool:
    """
    Cek apakah IP mengirim terlalu banyak request API (anti-flood).
    Dilengkapi garbage collection otomatis untuk mencegah kebocoran memori.
    """
    now = time.time()
    log = _api_request_log.get(ip, [])
    log = [t for t in log if now - t < _API_RATE_WINDOW_SEC]

    # Bersihkan entri lama jika log membesar (mencegah memory leak)
    if len(_api_request_log) > 500:
        stale_keys = [
            k for k, v in _api_request_log.items()
            if not v or (now - v[-1] >= _API_RATE_WINDOW_SEC)
        ]
        for k in stale_keys:
            _api_request_log.pop(k, None)

    if len(log) >= _API_RATE_MAX_REQUESTS:
        _api_request_log[ip] = log
        return True

    log.append(now)
    _api_request_log[ip] = log
    return False


def _is_valid_origin(request) -> bool:
    """
    Validasi Origin/Referer untuk CSRF protection pada POST/PUT/DELETE requests.
    Menggunakan parsing URL standar dengan validasi port integer ketat untuk mencegah spoofing.
    """
    origin = request.headers.get("Origin", "").strip()
    referer = request.headers.get("Referer", "").strip()

    # Jika client non-browser (misal internal script / curl), izinkan jika tidak ada origin/referer
    if not origin and not referer:
        return True

    valid_netlocs: Set[str] = set()
    valid_hostnames: Set[str] = set()

    # 1. Host dari header request saat ini
    host_header = request.headers.get("Host", "").strip().lower()
    if host_header:
        valid_netlocs.add(host_header)
        valid_hostnames.add(host_header.split(":")[0])

    # 2. Forwarded host dari reverse proxy (misal Railway, Nginx, Cloudflare)
    fwd_host = request.headers.get("X-Forwarded-Host", "").strip().lower()
    if fwd_host:
        valid_netlocs.add(fwd_host)
        valid_hostnames.add(fwd_host.split(":")[0])

    # 3. Host lokal yang selalu valid
    valid_hostnames.update({"localhost", "127.0.0.1"})
    valid_netlocs.update({
        "localhost",
        "127.0.0.1",
        f"localhost:{Config.WEB_PORT}",
        f"127.0.0.1:{Config.WEB_PORT}",
    })

    # Validasi candidate URL (Origin atau Referer)
    for candidate in (origin, referer):
        if not candidate:
            continue
        try:
            parsed = urlparse(candidate)
            # Validasi skema (hanya http dan https)
            if parsed.scheme not in ("http", "https"):
                continue

            # Validasi port: urlparse melempar ValueError jika ada karakter non-angka di port
            _ = parsed.port

            candidate_netloc = (parsed.netloc or "").lower()
            candidate_host = (parsed.hostname or "").lower()

            # Netloc cocok persis (misal 'localhost:8080' atau 'mybot.up.railway.app')
            if candidate_netloc in valid_netlocs:
                return True
            # Atau hostname cocok jika port tidak dispesifikasikan (default port 80/443 di reverse proxy)
            if parsed.port is None and candidate_host in valid_hostnames:
                return True
        except Exception:
            continue

    return False


def _is_protected() -> bool:
    """Kembalikan True jika DASHBOARD_PASSWORD sudah di-set."""
    from config import Config
    return bool(Config.DASHBOARD_PASSWORD.strip())


def _is_authenticated(request) -> bool:
    """Cek apakah request memiliki session token yang valid."""
    if not _is_protected():
        return True  # Tidak ada password → akses bebas
    token = request.cookies.get("ak_session")
    return token is not None and token in _valid_sessions


@web.middleware
async def auth_middleware(request, handler):
    """Middleware: kontrol akses, CSRF check, rate limiting, dan security headers."""
    PUBLIC_PATHS = {"/", "/api/auth/login", "/style.css", "/app.js"}
    path = request.path

    # Izinkan path publik dan file statis (CSS/JS/font/dll)
    is_static = path.startswith("/") and (
        path in PUBLIC_PATHS
        or path.endswith((".css", ".js", ".png", ".ico", ".woff2", ".woff", ".ttf"))
    )

    ip = get_client_ip(request)

    # Global API rate limiter (anti-flood) — berlaku untuk semua rute /api/
    if not is_static and path.startswith("/api/"):
        if _is_api_rate_limited(ip):
            logger.warning(f"Rate limit API terlampaui dari IP: {ip}, path: {path}")
            return web.json_response(
                {
                    "error": "Terlalu banyak permintaan (Rate limit). Silakan tunggu sebentar.",
                    "retry_after": _API_RATE_WINDOW_SEC,
                },
                status=429,
                headers={"Retry-After": str(_API_RATE_WINDOW_SEC)},
            )

    # CSRF check: validasi Origin/Referer untuk semua mutasi state (POST, PUT, DELETE, PATCH)
    if request.method in ("POST", "PUT", "DELETE", "PATCH") and path != "/api/auth/login":
        if not _is_valid_origin(request):
            logger.warning(f"Permintaan CSRF diblokir dari IP: {ip}, path: {path}")
            return web.json_response(
                {"error": "Permintaan tidak valid (CSRF origin check gagal)."},
                status=403,
            )

    if is_static or _is_authenticated(request):
        response = await handler(request)
    else:
        response = web.json_response(
            {"authenticated": False, "message": "Autentikasi diperlukan"},
            status=401,
        )

    # Tambahkan security headers ke semua response
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["X-Frame-Options"] = "DENY"
    response.headers["X-XSS-Protection"] = "1; mode=block"
    response.headers["Referrer-Policy"] = "strict-origin-when-cross-origin"
    response.headers["Cache-Control"] = "no-store"
    return response


class AntiKarbitApp:
    def __init__(self):
        self.client: Optional[TelegramClient] = None
        self.recognizer = None
        self.claimer = None
        self.listener: Optional[WaifuListener] = None
        self.me_info: Dict[str, Any] = {}
        self.sse_queues: Set[asyncio.Queue] = set()

    def broadcast_claim_event(self, event_data: Dict[str, Any]):
        """Kirim notifikasi deteksi waifu atau klaim ke Web SSE clients."""
        payload = f"event: claim_event\ndata: {json.dumps(event_data)}\n\n"
        for q in list(self.sse_queues):
            try:
                q.put_nowait(payload)
            except Exception:
                pass

    async def init_telegram(self):
        """Inisialisasi koneksi Telethon dan komponen bot."""
        errors = Config.validate()
        if errors:
            for err in errors:
                logger.error(f"Konfigurasi Error: {err}")
            return False

        logger.info("Menginisialisasi Telegram Client...")
        session_target = (
            StringSession(Config.TELEGRAM_STRING_SESSION)
            if Config.TELEGRAM_STRING_SESSION
            else Config.TELEGRAM_SESSION_NAME
        )
        self.client = TelegramClient(
            session_target,
            Config.TELEGRAM_API_ID,
            Config.TELEGRAM_API_HASH,
        )

        self.recognizer = get_recognizer(
            iqdb_min_sim=Config.IQDB_MIN_SIMILARITY,
            tracemoe_min_sim=Config.TRACEMOE_MIN_SIMILARITY,
        )

        self.claimer = Claimer(
            command_prefix=Config.CLAIM_COMMAND,
            name_format=Config.NAME_FORMAT,
            min_delay=Config.MIN_DELAY_SECONDS,
            max_delay=Config.MAX_DELAY_SECONDS,
            verify_timeout=Config.VERIFY_TIMEOUT_SECONDS,
            success_keywords=Config.SUCCESS_KEYWORDS,
            fail_keywords=Config.FAIL_KEYWORDS,
        )

        self.listener = WaifuListener(self.client, self.recognizer, self.claimer)
        self.listener.add_event_callback(self.broadcast_claim_event)
        self.listener.register()

        await self.client.start()  # type: ignore
        me = await self.client.get_me()
        self.me_info = {
            "name": getattr(me, "first_name", "Telegram User"),
            "username": getattr(me, "username", None),
            "id": getattr(me, "id", 0),
        }
        logger.info(f"Berhasil terhubung ke Telegram sebagai: {self.me_info['name']} (@{self.me_info['username']})")
        return True

    # --- HTTP Auth Routes ---
    async def handle_auth_login(self, request):
        """Endpoint login dashboard. Memvalidasi password dan menerbitkan cookie session."""
        from config import Config

        if not _is_protected():
            # Tidak ada password → langsung anggap login
            return web.json_response({"authenticated": True, "message": "Tidak ada password yang dikonfigurasi"})

        ip = get_client_ip(request)
        if _is_brute_forced(ip):
            logger.warning(f"Login diblokir (brute force) dari IP: {ip}")
            return web.json_response(
                {"authenticated": False, "message": "Terlalu banyak percobaan gagal. Tunggu 60 detik."},
                status=429,
            )

        try:
            data = await request.json()
            password = str(data.get("password", ""))
        except Exception:
            return web.json_response({"authenticated": False, "message": "Request tidak valid"}, status=400)

        expected_hash = _hash_password(Config.DASHBOARD_PASSWORD)
        given_hash = _hash_password(password)

        if secrets.compare_digest(expected_hash, given_hash):
            _clear_attempts(ip)
            token = _generate_session_token()
            _valid_sessions.add(token)
            logger.info(f"Login dashboard berhasil dari IP: {ip}")
            resp = web.json_response({"authenticated": True, "message": "Login berhasil"})
            resp.set_cookie(
                "ak_session",
                token,
                httponly=True,
                samesite="Strict",
                max_age=86400 * 7,  # 7 hari
            )
            return resp
        else:
            _record_failed_attempt(ip)
            remaining = _BRUTE_FORCE_MAX_ATTEMPTS - len(_login_attempts.get(ip, []))
            logger.warning(f"Percobaan login gagal dari IP: {ip} (sisa {remaining}x)")
            return web.json_response(
                {"authenticated": False, "message": f"Password salah. Sisa percobaan: {remaining}"},
                status=401,
            )

    async def handle_auth_logout(self, request):
        """Endpoint logout: hapus session token."""
        token = request.cookies.get("ak_session")
        if token:
            _valid_sessions.discard(token)
        resp = web.json_response({"success": True, "message": "Logout berhasil"})
        resp.del_cookie("ak_session")
        return resp

    async def handle_auth_check(self, request):
        """Cek status autentikasi saat ini."""
        return web.json_response({
            "authenticated": _is_authenticated(request),
            "protected": _is_protected(),
        })

    # --- HTTP Routes ---
    async def handle_index(self, request):
        index_path = os.path.join(os.path.dirname(__file__), "web", "index.html")
        return web.FileResponse(index_path)

    async def handle_status(self, request):
        mem_mb = get_current_memory_mb()
        stats = self.listener.stats if self.listener else {}
        is_active = self.listener.is_active if self.listener else False

        return web.json_response({
            "status": "online",
            "is_active": is_active,
            "memory_mb": mem_mb,
            "user": self.me_info,
            "stats": stats,
            "config": Config.as_dict(),
        })

    async def handle_toggle_bot(self, request):
        if not self.listener:
            return web.json_response({"success": False, "message": "Bot belum siap"}, status=400)

        self.listener.is_active = not self.listener.is_active
        state_str = "AKTIF" if self.listener.is_active else "DIJEDA (PAUSED)"
        logger.info(f"Bot listener diubah menjadi: {state_str}")
        return web.json_response({
            "success": True,
            "is_active": self.listener.is_active,
            "message": f"Bot {state_str}",
        })

    async def handle_settings_get(self, request):
        return web.json_response(Config.as_dict())

    async def handle_settings_post(self, request):
        try:
            data = await request.json()
            ok = Config.update_and_save(data)
            if ok:
                # Update claimer & recognizer instance jika berubah
                if self.claimer:
                    self.claimer.command_prefix = Config.CLAIM_COMMAND
                    self.claimer.name_format = Config.NAME_FORMAT
                    self.claimer.min_delay = Config.MIN_DELAY_SECONDS
                    self.claimer.max_delay = Config.MAX_DELAY_SECONDS
                    self.claimer.verify_timeout = Config.VERIFY_TIMEOUT_SECONDS
                if self.recognizer and hasattr(self.recognizer, "iqdb"):
                    self.recognizer.iqdb.min_similarity = Config.IQDB_MIN_SIMILARITY
                    self.recognizer.tracemoe.min_similarity = Config.TRACEMOE_MIN_SIMILARITY

                logger.info("Pengaturan bot berhasil diperbarui dari Web Dashboard.")
                return web.json_response({"success": True, "config": Config.as_dict()})
            else:
                return web.json_response({"success": False, "message": "Gagal menyimpan file .env"}, status=500)
        except Exception as e:
            return web.json_response({"success": False, "message": str(e)}, status=400)

    async def handle_test_image(self, request):
        """Endpoint untuk menguji deteksi gambar langsung dari browser."""
        if not self.recognizer:
            return web.json_response({"success": False, "message": "Engine recognizer belum siap"}, status=503)

        # 1. Batasi ukuran Content-Length jika dikirim oleh browser
        content_length = request.content_length or 0
        if content_length > _UPLOAD_MAX_BYTES:
            max_mb = _UPLOAD_MAX_BYTES // (1024 * 1024)
            return web.json_response(
                {"success": False, "message": f"Ukuran file terlalu besar! Maksimum {max_mb} MB."},
                status=413,
            )

        try:
            reader = await request.multipart()
            field = await reader.next()
            if not field or field.name != "image":
                return web.json_response({"success": False, "message": "Field 'image' tidak ditemukan"}, status=400)

            # Baca bytes dengan batasan ketat
            image_bytes = await field.read(size_limit=_UPLOAD_MAX_BYTES)
        except (ValueError, web.HTTPRequestEntityTooLarge):
            max_mb = _UPLOAD_MAX_BYTES // (1024 * 1024)
            return web.json_response(
                {"success": False, "message": f"Ukuran file melebihi batas {max_mb} MB."},
                status=413,
            )
        except Exception as e:
            return web.json_response({"success": False, "message": f"Gagal membaca file: {str(e)}"}, status=400)

        if not image_bytes:
            return web.json_response({"success": False, "message": "Gambar kosong"}, status=400)

        logger.info(f"Web Tester: Menerima uji gambar ({len(image_bytes)} bytes)...")
        char = await self.recognizer.identify(image_bytes)
        if not char:
            return web.json_response({
                "success": False,
                "message": "Karakter tidak berhasil dikenali di IQDB atau Trace.moe",
            })

        # Simulasi perintah klaim
        candidates = _build_name_candidates(char)
        primary_name = candidates[0] if candidates else char.full_name
        fallbacks = candidates[1:] if len(candidates) > 1 else []

        return web.json_response({
            "success": True,
            "character": {
                "full_name": char.full_name,
                "first_name": char.first_name,
                "last_name": char.last_name,
                "series": char.series or "Unknown",
                "confidence": round(char.confidence * 100, 1),
                "source": char.source,
            },
            "commands": {
                "primary": f"{Config.CLAIM_COMMAND} {primary_name}",
                "fallbacks": [f"{Config.CLAIM_COMMAND} {n}" for n in fallbacks],
            },
        })

    async def handle_events_sse(self, request):
        """Server-Sent Events (SSE) stream untuk logs & claim events real-time."""
        response = web.StreamResponse(
            status=200,
            reason="OK",
            headers={
                "Content-Type": "text/event-stream",
                "Cache-Control": "no-cache",
                "Connection": "keep-alive",
            },
        )
        await response.prepare(request)

        q = asyncio.Queue(maxsize=100)
        self.sse_queues.add(q)
        web_log_handler.subscribers.add(q)

        try:
            # Kirim log riwayat awal dari buffer
            for entry in list(web_log_handler.buffer):
                payload = f"event: log\ndata: {json.dumps(entry)}\n\n"
                await response.write(payload.encode("utf-8"))

            # Kirim event riwayat klaim dari buffer
            if self.listener:
                for event_data in list(self.listener.recent_events):
                    payload = f"event: claim_event\ndata: {json.dumps(event_data)}\n\n"
                    await response.write(payload.encode("utf-8"))

            # Loop membaca queue
            while True:
                item = await q.get()
                if isinstance(item, dict):
                    # log entry
                    payload = f"event: log\ndata: {json.dumps(item)}\n\n"
                else:
                    # formatted SSE payload
                    payload = str(item)

                await response.write(payload.encode("utf-8"))
        except (asyncio.CancelledError, ConnectionResetError):
            pass
        finally:
            self.sse_queues.discard(q)
            web_log_handler.subscribers.discard(q)

        return response


async def open_desktop_window(port: int):
    """Membuka antarmuka dalam mode jendela Desktop App mandiri jika di lingkungan Windows desktop."""
    if sys.platform != "win32" or os.getenv("RAILWAY_ENVIRONMENT") or os.getenv("DYNO"):
        return

    await asyncio.sleep(1.2)
    url = f"http://localhost:{port}"

    # Cek Microsoft Edge App Mode (tersedia bawaan di semua Windows 10/11)
    edge_paths = [
        os.path.expandvars(r"%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe"),
        os.path.expandvars(r"%ProgramFiles%\Microsoft\Edge\Application\msedge.exe"),
    ]
    # Cek Google Chrome App Mode
    chrome_paths = [
        os.path.expandvars(r"%ProgramFiles%\Google\Chrome\Application\chrome.exe"),
        os.path.expandvars(r"%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe"),
        os.path.expandvars(r"%LocalAppData%\Google\Chrome\Application\chrome.exe"),
    ]

    for p in edge_paths + chrome_paths:
        if os.path.exists(p):
            try:
                import subprocess
                subprocess.Popen([p, f"--app={url}"])
                logger.info(f"Antarmuka dibuka dalam mode Desktop Window ({os.path.basename(p)})")
                return
            except Exception:
                pass

    # Fallback ke browser standar
    import webbrowser
    webbrowser.open(url)


def create_web_app(app_instance: AntiKarbitApp) -> web.Application:
    app = web.Application(
        middlewares=[auth_middleware],
        client_max_size=_UPLOAD_MAX_BYTES,
    )
    app.router.add_get("/", app_instance.handle_index)
    # Auth
    app.router.add_post("/api/auth/login", app_instance.handle_auth_login)
    app.router.add_post("/api/auth/logout", app_instance.handle_auth_logout)
    app.router.add_get("/api/auth/check", app_instance.handle_auth_check)
    # Dashboard APIs
    app.router.add_get("/api/status", app_instance.handle_status)
    app.router.add_post("/api/bot/toggle", app_instance.handle_toggle_bot)
    app.router.add_get("/api/settings", app_instance.handle_settings_get)
    app.router.add_post("/api/settings", app_instance.handle_settings_post)
    app.router.add_post("/api/test-image", app_instance.handle_test_image)
    app.router.add_get("/api/events", app_instance.handle_events_sse)

    web_dir = os.path.join(os.path.dirname(__file__), "web")
    app.router.add_static("/", web_dir)
    return app


async def main():
    app_instance = AntiKarbitApp()

    # 1. Inisialisasi Telegram bot
    ok = await app_instance.init_telegram()
    if not ok:
        logger.error("Inisialisasi Telegram gagal. Pastikan .env terisi dengan benar.")
        return

    # 2. Setup Web Server (access_log dimatikan agar tidak membombardir log setiap beberapa detik)
    web_app = create_web_app(app_instance)
    runner = web.AppRunner(web_app, access_log=None)
    await runner.setup()

    host = Config.WEB_HOST
    port = Config.WEB_PORT
    site = web.TCPSite(runner, host, port)
    await site.start()

    logger.info("=" * 60)
    logger.info(f"🚀 ANTIKARBIT DASHBOARD AKTIF DI: http://localhost:{port}")
    logger.info("=" * 60)

    # 3. Buka jendela aplikasi desktop secara otomatis
    asyncio.create_task(open_desktop_window(port))

    # 4. Jalankan bot sampai Telethon terputus
    try:
        await app_instance.client.run_until_disconnected()  # type: ignore
    finally:
        await runner.cleanup()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except (KeyboardInterrupt, SystemExit):
        logger.info("Aplikasi dihentikan oleh pengguna.")
