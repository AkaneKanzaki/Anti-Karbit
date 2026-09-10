import os
import math
import logging
from typing import List, Dict, Any
from dotenv import load_dotenv  # type: ignore

logger = logging.getLogger("antikarbit.config")

# Muat variabel lingkungan dari file .env jika ada
load_dotenv()


class Config:
    # Telegram Credentials
    TELEGRAM_API_ID: int = int(os.getenv("TELEGRAM_API_ID", "0"))
    TELEGRAM_API_HASH: str = os.getenv("TELEGRAM_API_HASH", "")
    TELEGRAM_SESSION_NAME: str = os.getenv("TELEGRAM_SESSION_NAME", "waifu_claimer_session")
    TELEGRAM_STRING_SESSION: str = os.getenv("TELEGRAM_STRING_SESSION", "")

    # Dashboard Security
    DASHBOARD_PASSWORD: str = os.getenv("DASHBOARD_PASSWORD", "")

    # Reverse Image Search Settings (IQDB, Trace.moe, SauceNAO & Google Lens)
    IQDB_MIN_SIMILARITY: float = float(os.getenv("IQDB_MIN_SIMILARITY", "60.0"))
    _raw_tracemoe: float = float(os.getenv("TRACEMOE_MIN_SIMILARITY", "0.85"))
    TRACEMOE_MIN_SIMILARITY: float = _raw_tracemoe / 100.0 if _raw_tracemoe > 1.0 else _raw_tracemoe
    SAUCENAO_API_KEY: str = os.getenv("SAUCENAO_API_KEY", "").strip()
    SAUCENAO_MIN_SIMILARITY: float = float(os.getenv("SAUCENAO_MIN_SIMILARITY", "70.0"))
    LENS_ENABLED: bool = os.getenv("LENS_ENABLED", "true").lower() not in ("false", "0", "no")

    # Claim Settings
    CLAIM_COMMAND: str = os.getenv("CLAIM_COMMAND", "/protecc")
    NAME_FORMAT: str = os.getenv("NAME_FORMAT", "full").lower()  # 'first', 'full', 'both'

    # Trigger & Filters
    _trigger_raw: str = os.getenv(
        "TRIGGER_KEYWORDS",
        "A waifu has appeared!,A husbando has appeared!,/protecc name,Add her to your harem,Add him to your harem"
    )
    TRIGGER_KEYWORDS: List[str] = [k.strip() for k in _trigger_raw.split(",") if k.strip()]

    _chats_raw: str = os.getenv("TARGET_CHAT_IDS", "")
    TARGET_CHAT_IDS: List[int] = [
        int(c.strip()) for c in _chats_raw.split(",") if c.strip().lstrip("-").isdigit()
    ]

    # Delays
    MIN_DELAY_SECONDS: float = float(os.getenv("MIN_DELAY_SECONDS", "0.5"))
    MAX_DELAY_SECONDS: float = float(os.getenv("MAX_DELAY_SECONDS", "1.5"))

    # Claim Verification — deteksi balasan game bot setelah klaim dikirim
    VERIFY_TIMEOUT_SECONDS: float = float(os.getenv("VERIFY_TIMEOUT_SECONDS", "5.0"))

    # Kata kunci balasan game bot
    _success_raw: str = os.getenv(
        "SUCCESS_KEYWORDS",
        "now protected,added to your harem,added to your collection,is now yours,congratulations"
    )
    SUCCESS_KEYWORDS: List[str] = [k.strip().lower() for k in _success_raw.split(",") if k.strip()]

    _fail_raw: str = os.getenv(
        "FAIL_KEYWORDS",
        "not quite right,wrong name,already claimed,already protecc,rip,try again"
    )
    FAIL_KEYWORDS: List[str] = [k.strip().lower() for k in _fail_raw.split(",") if k.strip()]

    # Web Dashboard Port (Mendukung PORT dari Railway/Heroku/Render)
    WEB_PORT: int = int(os.getenv("PORT", os.getenv("WEB_PORT", "8080")))
    WEB_HOST: str = os.getenv("WEB_HOST", "0.0.0.0")

    @classmethod
    def validate(cls) -> List[str]:
        """Memvalidasi kelengkapan konfigurasi penting."""
        errors = []
        if not cls.TELEGRAM_API_ID or cls.TELEGRAM_API_ID == 0:
            errors.append("TELEGRAM_API_ID belum diisi di .env")
        if not cls.TELEGRAM_API_HASH:
            errors.append("TELEGRAM_API_HASH belum diisi di .env")
        return errors

    @classmethod
    def as_dict(cls) -> Dict[str, Any]:
        """Mengembalikan konfigurasi dalam bentuk dictionary untuk Web UI."""
        return {
            "CLAIM_COMMAND": cls.CLAIM_COMMAND,
            "NAME_FORMAT": cls.NAME_FORMAT,
            "IQDB_MIN_SIMILARITY": cls.IQDB_MIN_SIMILARITY,
            "TRACEMOE_MIN_SIMILARITY": cls.TRACEMOE_MIN_SIMILARITY,
            "SAUCENAO_API_KEY": cls.SAUCENAO_API_KEY,
            "SAUCENAO_MIN_SIMILARITY": cls.SAUCENAO_MIN_SIMILARITY,
            "LENS_ENABLED": cls.LENS_ENABLED,
            "TRIGGER_KEYWORDS": ", ".join(cls.TRIGGER_KEYWORDS),
            "TARGET_CHAT_IDS": ", ".join(map(str, cls.TARGET_CHAT_IDS)),
            "MIN_DELAY_SECONDS": cls.MIN_DELAY_SECONDS,
            "MAX_DELAY_SECONDS": cls.MAX_DELAY_SECONDS,
            "VERIFY_TIMEOUT_SECONDS": cls.VERIFY_TIMEOUT_SECONDS,
            "SUCCESS_KEYWORDS": ", ".join(cls.SUCCESS_KEYWORDS),
            "FAIL_KEYWORDS": ", ".join(cls.FAIL_KEYWORDS),
            "WEB_PORT": cls.WEB_PORT,
            "WEB_HOST": cls.WEB_HOST,
        }

    @classmethod
    def update_and_save(cls, new_settings: Dict[str, Any]) -> bool:
        """Memperbarui atribut class dan menyimpan ke file .env secara aman."""
        try:
            def _to_float(val: Any, default: float) -> float:
                if val is None or val == "":
                    return default
                try:
                    f = float(val)
                    return default if math.isnan(f) else f
                except (ValueError, TypeError):
                    return default

            if "CLAIM_COMMAND" in new_settings and new_settings["CLAIM_COMMAND"]:
                cls.CLAIM_COMMAND = str(new_settings["CLAIM_COMMAND"]).strip()
            if "NAME_FORMAT" in new_settings and new_settings["NAME_FORMAT"]:
                cls.NAME_FORMAT = str(new_settings["NAME_FORMAT"]).strip().lower()
            if "IQDB_MIN_SIMILARITY" in new_settings:
                cls.IQDB_MIN_SIMILARITY = _to_float(new_settings["IQDB_MIN_SIMILARITY"], cls.IQDB_MIN_SIMILARITY)
            if "TRACEMOE_MIN_SIMILARITY" in new_settings:
                t_val = _to_float(new_settings["TRACEMOE_MIN_SIMILARITY"], cls.TRACEMOE_MIN_SIMILARITY)
                # Jika user memasukkan format persen (misal 80 atau 85), ubah ke desimal 0.80 atau 0.85
                cls.TRACEMOE_MIN_SIMILARITY = t_val / 100.0 if t_val > 1.0 else t_val
            if "SAUCENAO_API_KEY" in new_settings:
                cls.SAUCENAO_API_KEY = str(new_settings["SAUCENAO_API_KEY"] or "").strip()
            if "SAUCENAO_MIN_SIMILARITY" in new_settings:
                cls.SAUCENAO_MIN_SIMILARITY = _to_float(new_settings["SAUCENAO_MIN_SIMILARITY"], cls.SAUCENAO_MIN_SIMILARITY)
            if "LENS_ENABLED" in new_settings:
                v = new_settings["LENS_ENABLED"]
                cls.LENS_ENABLED = str(v).lower() not in ("false", "0", "no") if isinstance(v, str) else bool(v)
            if "TRIGGER_KEYWORDS" in new_settings:
                raw = str(new_settings["TRIGGER_KEYWORDS"] or "")
                kw_list = [k.strip() for k in raw.split(",") if k.strip()]
                if kw_list:
                    cls.TRIGGER_KEYWORDS = kw_list
            if "TARGET_CHAT_IDS" in new_settings:
                raw = str(new_settings["TARGET_CHAT_IDS"] or "")
                cls.TARGET_CHAT_IDS = [
                    int(c.strip()) for c in raw.split(",") if c.strip().lstrip("-").isdigit()
                ]
            if "MIN_DELAY_SECONDS" in new_settings:
                cls.MIN_DELAY_SECONDS = _to_float(new_settings["MIN_DELAY_SECONDS"], cls.MIN_DELAY_SECONDS)
            if "MAX_DELAY_SECONDS" in new_settings:
                cls.MAX_DELAY_SECONDS = _to_float(new_settings["MAX_DELAY_SECONDS"], cls.MAX_DELAY_SECONDS)
            if "VERIFY_TIMEOUT_SECONDS" in new_settings:
                cls.VERIFY_TIMEOUT_SECONDS = _to_float(new_settings["VERIFY_TIMEOUT_SECONDS"], cls.VERIFY_TIMEOUT_SECONDS)

            # Baca file .env lama untuk mempertahankan TELEGRAM credentials
            env_path = os.path.join(os.path.dirname(__file__), ".env")
            env_lines = []
            if os.path.exists(env_path):
                with open(env_path, "r", encoding="utf-8") as f:
                    env_lines = f.readlines()

            keys_to_update = {
                "CLAIM_COMMAND": cls.CLAIM_COMMAND,
                "NAME_FORMAT": cls.NAME_FORMAT,
                "IQDB_MIN_SIMILARITY": str(cls.IQDB_MIN_SIMILARITY),
                "TRACEMOE_MIN_SIMILARITY": str(cls.TRACEMOE_MIN_SIMILARITY),
                "SAUCENAO_API_KEY": cls.SAUCENAO_API_KEY,
                "SAUCENAO_MIN_SIMILARITY": str(cls.SAUCENAO_MIN_SIMILARITY),
                "LENS_ENABLED": str(cls.LENS_ENABLED).lower(),
                "TRIGGER_KEYWORDS": ",".join(cls.TRIGGER_KEYWORDS),
                "TARGET_CHAT_IDS": ",".join(map(str, cls.TARGET_CHAT_IDS)),
                "MIN_DELAY_SECONDS": str(cls.MIN_DELAY_SECONDS),
                "MAX_DELAY_SECONDS": str(cls.MAX_DELAY_SECONDS),
                "VERIFY_TIMEOUT_SECONDS": str(cls.VERIFY_TIMEOUT_SECONDS),
            }

            new_lines = []
            handled_keys = set()
            for line in env_lines:
                stripped = line.strip()
                if "=" in stripped and not stripped.startswith("#"):
                    k = stripped.split("=", 1)[0].strip()
                    if k in keys_to_update:
                        new_lines.append(f"{k}={keys_to_update[k]}\n")
                        handled_keys.add(k)
                        continue
                new_lines.append(line)

            for k, v in keys_to_update.items():
                if k not in handled_keys:
                    new_lines.append(f"{k}={v}\n")

            with open(env_path, "w", encoding="utf-8") as f:
                f.writelines(new_lines)

            logger.info("Konfigurasi bot berhasil diperbarui dan disimpan ke .env")
            return True
        except Exception as e:
            logger.error(f"Gagal memperbarui konfigurasi: {e}", exc_info=True)
            return False
