from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Optional, List

@dataclass
class CharacterInfo:
    full_name: str
    first_name: str
    last_name: Optional[str] = None
    series: Optional[str] = None
    confidence: float = 1.0
    source: str = "unknown"
    # Nama-nama karakter alternatif dari engine (digunakan sebagai fallback klaim)
    # Misalnya TraceMoe mengembalikan list karakter dari AniList, dan semua akan dicoba
    alternate_names: List[str] = field(default_factory=list)

    def get_claim_names(self, mode: str = "full") -> List[str]:
        """
        Mengembalikan daftar nama yang akan dikirim untuk klaim berdasarkan preferensi mode:
        - 'first': hanya nama depan (contoh: Mikan)
        - 'full': hanya nama lengkap (contoh: Mikan Tsumiki)
        - 'both': kirim nama depan dan nama lengkap
        """
        names = []
        if mode == "first":
            if self.first_name:
                names.append(self.first_name)
            else:
                names.append(self.full_name)
        elif mode == "both":
            if self.first_name:
                names.append(self.first_name)
            if self.full_name and self.full_name != self.first_name:
                names.append(self.full_name)
        else: # 'full'
            names.append(self.full_name)
            
        return names

class BaseRecognizer(ABC):
    @abstractmethod
    async def identify(self, image_bytes: bytes) -> Optional[CharacterInfo]:
        """
        Menganalisis byte gambar dan mengembalikan informasi karakter jika ditemukan.
        """
        pass
