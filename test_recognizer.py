import sys
import asyncio
import os

# Pastikan output konsol Windows mendukung karakter Unicode
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

from config import Config
from recognizer import get_recognizer


async def test_image(image_path: str):
    if not os.path.exists(image_path):
        print(f"File gambar tidak ditemukan: {image_path}")
        return

    print(f"\n[1] Membaca gambar dari: {image_path}")
    with open(image_path, "rb") as f:
        img_bytes = f.read()

    print("[2] Inisialisasi recognizer pencarian internet (IQDB + Trace.moe)...")
    recognizer = get_recognizer(
        iqdb_min_sim=Config.IQDB_MIN_SIMILARITY,
        tracemoe_min_sim=Config.TRACEMOE_MIN_SIMILARITY,
    )

    print("[3] Mengirim ke mesin pencari internet (IQDB / Trace.moe)...")
    result = await recognizer.identify(img_bytes)

    if not result:
        print("\n[HASIL GAGAL] Karakter tidak dapat ditemukan di IQDB maupun Trace.moe.")
        return

    print("\n================ HASIL PENGENALAN ================")
    print(f"Nama Lengkap  : {result.full_name}")
    print(f"Nama Depan    : {result.first_name}")
    print(f"Nama Belakang : {result.last_name}")
    print(f"Asal Seri     : {result.series or 'Unknown'}")
    print(f"Confidence    : {result.confidence * 100:.1f}%")
    print(f"Engine Sumber : {result.source}")
    print("--------------------------------------------------")
    print("Contoh Perintah Klaim yang Dihasilkan:")
    for mode in ["first", "full", "both"]:
        names = result.get_claim_names(mode=mode)
        cmds = [f"{Config.CLAIM_COMMAND} {n}" for n in names]
        print(f" - Mode '{mode:<5}': {' dan '.join(cmds)}")
    print("==================================================\n")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Penggunaan: python test_recognizer.py <path_ke_gambar.jpg>")
        print("Contoh: python test_recognizer.py test_waifu.png")
        sys.exit(1)

    asyncio.run(test_image(sys.argv[1]))
