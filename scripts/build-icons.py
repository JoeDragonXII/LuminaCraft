"""Package the approved v9 artwork into native icon sizes (requires Pillow)."""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parents[1]
destination = root / "assets" / "app-icon"
destination.mkdir(exist_ok=True)
with Image.open(root / "assets/branding/lumina-shared-icon-v9-landscape.png") as source:
    artwork = source.convert("RGBA")
    artwork.resize((256, 256), Image.Resampling.LANCZOS).save(destination / "icon-256.png")
    artwork.save(destination / "luminacraft.ico", sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
print(destination)
