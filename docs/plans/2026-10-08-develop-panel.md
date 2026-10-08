# Panel wywoływania zdjęć w tx (port termilight do Rusta)

Data: 2026-10-08

## Cel

`<cr>` na zdjęciu (JPEG/PNG/TIFF/RAW) otwiera w kolumnie podglądu panel edycji jak w termilight:
podgląd na żywo, histogram RGB, suwaki w panelach Basic / Krzywa / HSL / Detal / Kadr, eksport
w pełnej rozdzielczości do `<nazwa>_edit.jpg` (nigdy nie nadpisuje). Ustawienia giną po wyjściu
(bez plików-sidecar), tak jak w termilight.

Poza zakresem (jak w termilight): katalog, edycja wsadowa, krzywe per kanał, maski.

## Co już jest w tx i co bierzemy

| Potrzeba | Jest w tx | Użycie |
|---|---|---|
| Tryb przejmujący klawisze | `Mode::Edit(Box<Editor>)` w `app.rs` | nowy `Mode::Develop(Box<Develop>)` obok |
| Rysowanie obrazu (kitty/iTerm2/sixel/bloki, SSH) | `Painter::draw(path, ImageData, area)` | bez zmian. Klucz cache to `(path, Arc obrazu, size)`, a `store()` wyrzuca stare wersje tej samej ścieżki, a do czasu zakodowania nowej klatki zostaje poprzednia, więc podgląd nie miga |
| Kodowanie poza głównym wątkiem | `spawn_encoder` w `runtime.rs` | bez zmian |
| Rozmiar proxy | `Painter::decode_target(cols, rows)` | proxy = dokładnie tyle pikseli, ile pokazuje kolumna (zamiast stałego 900 px) |
| Histogram luminancji, karty tekstu | `preview::histogram`, `preview::card` | rozszerzyć o R/G/B (kolorowe `Span`) |
| Dekodowanie JPEG/PNG/TIFF + EXIF orientacja | `preview::decode_*`, `jpeg_orientation` | pełna rozdzielczość: te same funkcje bez limitu rozmiaru |
| Zapis bezpieczny | `save::write_file` | eksport przez nie (atomowo) |
| JPEG q92 / TIFF 16-bit | `image` z cechami `jpeg`, `tiff` | `tifffile` z Pythona niepotrzebny |
| Równoległość | `rayon` już jest w `Cargo.lock` (przez `image`) | dodać jako bezpośrednią zależność, `par_chunks_mut` po wierszach |

## Jedyna nowa zależność: RAW

Dziś tx pokazuje z RAW tylko osadzony JPEG (`rawfile.rs`). Do edycji trzeba prawdziwych danych
liniowych, bo osadzony JPEG to 8-bit sRGB i nie ma zapasu w światłach.

- **`rawler`** (z dnglab, czysty Rust): CR2/CR3/NEF/ARW/DNG/RAF/ORF/RW2, demozaikowanie, WB z aparatu,
  macierz kamera→sRGB. Odpowiednik `rawpy.postprocess(gamma=(1,1), use_camera_wb=True)`.
- Krok 1 planu to spike: sprawdzić API `rawler` (develop → liniowe f32 RGB), czas na DSCF4261.JPG-owym
  RAF-ie i rozmiar binarki w `--profile dist`. Jeśli `rawler` się nie sprawdzi: v1 edytuje osadzony
  pełnowymiarowy JPEG (zero nowych zależności, mniejszy zapas), RAW dochodzi później.
- Alternatywa tylko na macOS: ImageIO (`imageio.rs`, już linkowane) umie wywołać RAW przez
  `CGImageSourceCreateImageAtIndex`. Na Linuksie nie zadziała, więc to co najwyżej przyspieszenie.

### Wyniki kroku 1 (spike, 2026-10-08)

Program testowy w scratchpadzie: `rawler 0.8.0`, `RawDevelop` z krokami Rescale, Demosaic, FujiRotate,
CropActiveArea, WhiteBalance, Calibrate i CropDefault, czyli wszystko poza `SRgb`. Wynik to liniowe f32
RGB w primaries sRGB z WB z aparatu, tak jak `rawpy.postprocess(gamma=(1,1), no_auto_bright, camera_wb)`.
Wartości > 1 zostają (max 1.2–1.5). Czasy z release, M-seria, pliki już na dysku:

| plik | wymiary | dekodowanie | wywołanie | średnia rawler / rawpy |
|---|---|---|---|---|
| RAF, X100VI (X-Trans) | 7728×5152 | 95 ms | 1.0–1.1 s | 0.0230 / 0.0222 |
| NEF, Z 6 | 6048×4024 | 213 ms | 0.3–0.5 s | 0.0983 / 0.0924 |
| DNG, DJI FC3170 | 4000×2250 | 48 ms | 0.11–0.17 s | 0.1332 / 0.1331 |
| DNG, Lightroom Enhanced-NR (liniowy) | 4896×3264 | 59 ms | 0.18–0.25 s | 0.0974 / 0.0933 |
| CR2 mRAW, 6D Mark II | 4680×3120 | 300 ms | 0.3 s | **0.0008 / 0.011** |

Dla porównania `rawpy` potrzebuje na ten RAF 18 s, a na NEF 1.5 s. Pierwsze pomiary DNG (8–11 s)
wynikały z iCloud (`~/Documents` ściągał pliki przy pierwszym odczycie), nie z `rawler`.

Znalezione problemy i co z nimi robimy:
- **Orientacja**: `rawler` zwraca `Normal` dla RAF-a robionego w pionie, a `rawpy` zna flip 5.
  Orientację bierzemy z istniejącego `preview::part_orientation`, który już działa dla podglądu RAW
  (`image::Orientation`), a nie z `rawler`.
- **Canon mRAW/sRAW** (`camera.mode` zaczyna się od `sRaw`): `rawler` przyjmuje punkt bieli 65000,
  a dane sięgają ok. 15000, więc obraz jest 14× za ciemny. Zwykłe CR2 (`cpp 1`) idą inną ścieżką.
  Te tryby wywołujemy z osadzonego JPEG-a (droga z planu B) i zgłaszamy błąd do dnglab.
- **Rozmiar binarki**: `dist` z 5.13 MB do 8.57 MB (+3.4 MB, +67%) i 82 nowe crate'y (227 → 309),
  m.in. `jxl-oxide`, `toml 0.8` obok naszego 1.x i baza aparatów. `rawler` nie ma cech, które dałoby się
  wyłączyć. Czas buildu `dist`: 77 s → 102 s.
- **Pamięć**: wywołanie 40 Mpx RAF-a to szczyt 1.3 GB RSS (`develop_intermediate` klonuje surowy obraz).
  Akceptujemy to; Pi jest poza zakresem tej funkcji.
- **Licencja**: `rawler` jest na LGPL-2.1, a Rust linkuje statycznie. tx jest teraz na MIT (`LICENSE`,
  `license = "MIT"` w Cargo.toml), a jego źródła są otwarte, więc każdy może przebudować go ze zmienioną
  biblioteką. To spełnia wymóg LGPL. Przy wydawaniu binarek w README trzeba wspomnieć o `rawler` (LGPL).

**Decyzja**: `rawler` jako zwykła zależność, bez cechy Cargo. Orientacja z tx, mRAW przez osadzony JPEG.

## Nowe moduły

```
src/develop/
  pipeline.rs   Settings + process(&Image32, &Settings, scale) -> Image32 (sRGB [0,1])
  geometry.rs   rot90, straighten (bilinearnie + wpisany prostokąt), crop, fit_aspect, move/resize_crop
  load.rs       load(path) -> liniowe f32 RGB (rawler / image + orientacja EXIF + ICC→sRGB), export()
  mod.rs        struct Develop: stan panelu, klawisze, undo/redo; jak Editor zwraca DevelopEvent
```

`Image32 { w, h, px: Vec<[f32; 3]> }`, bez `ndarray`: cały pipeline to pętle po pikselach plus trzy filtry.

### Port pipeline.py → Rust (1:1, ta sama kolejność i te same stałe)

| Python (numpy/scipy) | Rust |
|---|---|
| `srgb_to_linear` / `linear_to_srgb` | per piksel; dla wejścia 8-bit LUT 256 |
| `white_balance`, ekspozycja, `contrast`, `tone` | per piksel, `rayon` po wierszach |
| `PchipInterpolator` (5 punktów) | ok. 25 linii: pochodne Fritscha–Carlsona + Hermite, LUT 4096 |
| `hsl` (LUT 1024 wag pasm) | tak samo, `rgb_to_hsv`/`hsv_to_rgb` per piksel |
| `vibrance_saturation` | per piksel |
| `gaussian_filter` / 3× `uniform_filter` | 3× box blur (suma bieżąca, rozdzielnie w x i y), działa dla każdej sigmy |
| `scipy.ndimage.rotate` | bilinearne próbkowanie wstecz, tylko w wycinanym prostokącie |
| `np.histogram` | 3 tablice `[u32; BINS]` |

Każdy krok pomija się przy wartości neutralnej, tak jak w termilight, więc domyślne ustawienia dają tożsamość.

## Wpięcie w tx

**app.rs**
- `edit_here(path)`: jeśli `preview` rozpozna obraz (ta sama detekcja co `preview.rs:134`, wydzielona
  do `fn is_picture(head, path)`), to `Mode::Develop`, w przeciwnym razie edytor tekstu jak dziś.
- `press()`: gałąź `Mode::Develop(_) => self.press_develop(key)`.
- `develop()` / `take_develop_request()` dla runtime i renderera, na wzór `editor()` / `take_jobs()`.

**runtime.rs**
- `spawn_developer(tx)`: jeden wątek, kanał z zasadą „ostatnie żądanie wygrywa” (przed pracą
  `try_iter().last()`). Żądanie = `(gen, Arc<Image32> proxy, Settings)`. Odpowiedź
  `Msg::Developed(gen, ImageData, Histogram)`; app odrzuca `gen` starsze niż bieżące.
- Debounce 50 ms: `wait` dostaje `app.develop_pending().then_some(DEVELOP_DEBOUNCE)`, tak jak
  `HIGHLIGHT_IDLE` dla edytora.
- Ładowanie proxy w tle (dekodowanie i od razu zmniejszenie, jak `decode_jpeg_scaled`):
  `Msg::DevelopLoaded(path, Result<Image32>)`. Do tego czasu panel
  pokazuje dotychczasowy podgląd z `preview` (osadzony JPEG), więc nic nie miga.
- Eksport: `thread::spawn`, `Msg::Exported(path, Result<()>)`. Ścieżki zarezerwowane w głównym wątku,
  jak `reserved` w termilight.

**render.rs**
- `draw_develop(buf, tree, center, p, develop, painter)` obok `draw_editor`: obraz po lewej
  (`painter.draw(develop.path(), &develop.shown, …)`), po prawej panel o szerokości ok. 40 kolumn:
  histogram RGB, zakładki, suwaki `━━━●───`, wykres krzywej brajlem. Przy wąskiej kolumnie panel idzie
  pod obraz; tę decyzję podejmuje już `picture_layout()` dla karty zdjęcia, więc go używamy.
- Kadr: przyciemnienie poza ramką na proxy przed wysłaniem do `Painter`, jak `dim_outside`.
- Stopka: `draw_develop_footer` na wzór `draw_editor_footer` (panel, `[+]`, komunikat, status eksportu).

**keys**: tryb bierze wszystkie klawisze jak edytor, więc keymap tx zostaje nietknięty.

| Klawisz | Akcja |
|---|---|
| `<tab>` `<s-tab>` | panel |
| `j` `k` / `↓` `↑` | suwak (Kadr: przesuń ramkę) |
| `h` `l` / `←` `→`, `H` `L` | wartość ±krok / ±10 kroków (Kadr: `H` `J` `K` `L` zmieniają rozmiar) |
| `0` | reset suwaka / kadru |
| `u` `<c-r>` | cofnij / ponów |
| `\` | przed / po |
| `w` | eksport |
| `[` `]` `a` `r` | pasmo HSL; prostowanie, proporcje, obrót |
| `q` `<esc>` | wyjście; przy niezapisanych zmianach drugie `q` |

Strzałki działają tak samo jak w termilight; `hjkl` dla spójności z resztą tx.

## Kolejność prac (każdy krok to osobny commit z testem)

1. **Spike RAW**: `rawler` na RAF/CR3/NEF, czas, rozmiar binarki. Decyzja: rawler albo osadzony JPEG w v1.
2. **`develop/pipeline.rs` + `geometry.rs`**: port z testami przepisanymi z `tests/test_pipeline.py`
   (tożsamość dla domyślnych, +1 EV = ×2 liniowo, WB, kontrast trzyma 18% szarości, cienie, krzywa
   monotoniczna, kadr 1:1 z 300×200 → 200×200, rot90 zamienia wymiary). Benchmark w `examples/`:
   cel < 50 ms dla proxy ok. 1500×1000 na M-serii (termilight: ok. 160 ms przy 900 px).
3. **`develop/load.rs`**: wczytanie + eksport, `unique_path`, testy z `test_io.py`.
4. **`Develop` (stan + klawisze)**: czysta logika bez UI, testy jak `test_app.py` (undo/redo, reset,
   zmiana panelu, podwójne `q`).
5. **Wpięcie w `app.rs` + `runtime.rs`**: tryb, wątek wywołujący, debounce, ładowanie w tle, eksport.
   Test w stylu `i_opens_the_file_under_the_cursor_in_the_editor…`.
6. **`render.rs`**: panel, histogram RGB, krzywa, kadr. Testy bufora jak w `render.rs`.
7. **Ręcznie**: kitty i iTerm2, lokalnie i przez SSH; README (klawisze) i `?`.

## Ryzyka

- **Przepustowość terminala przy przeciąganiu suwaka**: każda klatka to nowy obraz (kitty: upload,
  iTerm2: JPEG). Debounce 50 ms plus „ostatnie wygrywa” w wątku wywołującym i w enkoderze. Przez SSH
  `Painter::remote` już zmniejsza obraz.
- **RAW**: jakość i szybkość `rawler` do sprawdzenia w kroku 1; plan B opisany wyżej.
- **Pamięć**: pełny RAW 40 Mpx w f32 RGB to ok. 480 MB. Trzymamy pełny obraz tylko na czas eksportu
  (wczytujemy go ponownie z pliku), a w trybie edycji tylko proxy.
