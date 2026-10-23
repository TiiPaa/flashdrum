"""Génère les cinq textures longues de l'instrument Rift ([221], [291]).

Ces WAV sont embarqués dans le binaire (`include_bytes!`), donc ils sont
fabriqués ici plutôt que sourcés : contenu original, aucune licence à gérer, et
un rendu reproductible — la graine est fixe, relancer le script redonne
exactement les mêmes fichiers.

    python tools/gen_textures.py

Sortie : `drum-pattern-vst/assets/textures/*.wav`, mono, PCM 16 bits, 44 100 Hz,
12 s chacune (~1,06 Mo par fichier), 16 s pour le champ de bruit. Le parseur RIFF du plugin
(`src/synthesis/sample_bank.rs`) lit ce format tel quel.

La consigne commune aux cinq : **la matière doit changer de caractère au fil
des secondes**. Rift prélève une tranche à un offset choisi — si la texture est
uniforme, déplacer l'offset ne produit rien d'audible et l'instrument perd sa
raison d'être.
"""

import os
import struct
import wave

import numpy as np

SR = 44100
DURATION = 12.0
N = int(SR * DURATION)
PEAK = 0.89  # marge : la voix ajoute filtre, saturation et volume par-dessus

OUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..",
    "drum-pattern-vst",
    "assets",
    "textures",
)


# ---------------------------------------------------------------- utilitaires


def rng(seed):
    return np.random.default_rng(seed)


def drift(r, n, hz_list, depths):
    """Somme de sinus lents, normalisée dans [0, 1] — une dérive douce et
    apériodique (les fréquences sont incommensurables, le motif ne se répète
    pas sur la durée du fichier)."""
    t = np.arange(n) / SR
    out = np.zeros(n)
    for hz, depth in zip(hz_list, depths):
        out += depth * np.sin(2 * np.pi * hz * t + r.uniform(0, 2 * np.pi))
    lo, hi = out.min(), out.max()
    return (out - lo) / max(hi - lo, 1e-9)


def onepole_lp_varying(x, cutoff):
    """Passe-bas 1 pôle dont la fréquence de coupure bouge à chaque sample."""
    a = np.exp(-2.0 * np.pi * np.clip(cutoff, 20.0, SR * 0.45) / SR)
    y = np.empty_like(x)
    z = 0.0
    for i in range(len(x)):
        z = (1.0 - a[i]) * x[i] + a[i] * z
        y[i] = z
    return y


def svf_varying(x, cutoff, q, out="bp"):
    """Filtre à variable d'état 2 pôles (topologie TPT / ZDF), cutoff et Q
    modulables par sample. Renvoie la sortie passe-bande (`out="bp"`, la plus
    parlante sur du bruit) ou passe-bas (`out="lp"`, 12 dB/oct : un pôle seul
    ne rend jamais un bruit vraiment sourd, son centroïde reste à plusieurs
    kilohertz quelle que soit la coupure).

    La forme naïve de Chamberlin diverge dès que la coupure dépasse SR/6 avec
    un Q élevé — ce qui est exactement le régime du balayage. Celle-ci est
    inconditionnellement stable.
    """
    g = np.tan(np.pi * np.clip(cutoff, 20.0, SR * 0.45) / SR)
    k = 1.0 / np.clip(q, 0.5, 40.0)
    a1 = 1.0 / (1.0 + g * (g + k))
    a2 = g * a1
    a3 = g * a2
    ic1 = ic2 = 0.0
    y = np.empty_like(x)
    lp = out == "lp"
    for i in range(len(x)):
        v3 = x[i] - ic2
        v1 = a1[i] * ic1 + a2[i] * v3
        v2 = ic2 + a2[i] * ic1 + a3[i] * v3
        ic1 = 2.0 * v1 - ic1
        ic2 = 2.0 * v2 - ic2
        y[i] = v2 if lp else v1
    return y


def normalize(x):
    peak = np.max(np.abs(x))
    return x * (PEAK / peak) if peak > 1e-9 else x


def fade_edges(x, ms=8.0):
    """Fondu aux deux bouts : un offset proche du début ou de la fin ne doit pas
    attaquer sur un saut de niveau."""
    n = int(SR * ms / 1000.0)
    ramp = np.linspace(0.0, 1.0, n)
    x[:n] *= ramp
    x[-n:] *= ramp[::-1]
    return x


def write_wav(name, data):
    os.makedirs(OUT_DIR, exist_ok=True)
    path = os.path.normpath(os.path.join(OUT_DIR, name))
    pcm = np.clip(data, -1.0, 1.0)
    pcm = (pcm * 32767.0).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    rms = float(np.sqrt(np.mean(data**2)))
    print(f"  {name:<22} {os.path.getsize(path):>8} o   pic {np.max(np.abs(data)):.3f}   RMS {rms:.3f}")
    return path


# ------------------------------------------------------------------- textures


NOISE_DURATION = 16.0


def level_flat(x, window_ms=120.0):
    """Ramène le niveau RMS glissant à une constante : la matière peut changer
    de couleur, de rugosité, de hauteur, mais jamais de volume. Fenêtre de
    120 ms (Hann, symétrique) : assez courte pour rattraper une dérive, assez
    longue pour laisser vivre le grain et la modulation d'amplitude rapide."""
    n = max(int(SR * window_ms / 1000.0), 3)
    win = np.hanning(n)
    win /= win.sum()
    power = np.convolve(x * x, win, mode="same")
    env = np.sqrt(np.maximum(power, 1e-12))
    return x / np.maximum(env, 1e-4)


def highpass(x, hz, poles=4):
    """Passe-haut a 1 pole repete `poles` fois (6 dB/oct chacun)."""
    a = np.exp(-2.0 * np.pi * hz / SR)
    for _ in range(poles):
        y = np.empty_like(x)
        prev_x = 0.0
        prev_y = 0.0
        for i in range(len(x)):
            prev_y = a * (prev_y + x[i] - prev_x)
            prev_x = x[i]
            y[i] = prev_y
        x = y
    return x


def level_flat_heard(x, window_ms=120.0, weight_hz=400.0):
    """Comme `level_flat`, mais le niveau est mesure sur ce que l'oreille
    entend : la version passee au passe-haut (2 poles a `weight_hz`), qui
    neglige le grave comme le fait l'oreille a faible niveau. Une zone sourde
    n'est plus remontee au meme RMS qu'une zone brillante (elle sonnait alors
    plus faible), elle est remontee au meme niveau ENTENDU."""
    heard = highpass(x, weight_hz, poles=2)
    n = max(int(SR * window_ms / 1000.0), 3)
    win = np.hanning(n)
    win /= win.sum()
    power = np.convolve(heard * heard, win, mode="same")
    env = np.sqrt(np.maximum(power, 1e-12))
    return x / np.maximum(env, 1e-4)


def comb_varying(x, delay_ms, feedback):
    """Comb à délai variable (interpolation linéaire) avec réinjection : les
    résonances harmoniques d'un tuyau, dont la longueur bouge."""
    max_d = int(SR * 0.012) + 2
    buf = np.zeros(max_d)
    out = np.empty_like(x)
    w = 0
    d = np.clip(delay_ms * SR / 1000.0, 1.0, max_d - 2.0)
    for i in range(len(x)):
        rp = w - d[i]
        i0 = int(np.floor(rp)) % max_d
        i1 = (i0 + 1) % max_d
        frac = rp - np.floor(rp)
        delayed = buf[i0] * (1.0 - frac) + buf[i1] * frac
        y = x[i] + feedback[i] * delayed
        buf[w] = y
        w = (w + 1) % max_d
        out[i] = y
    return out


def noise_field(seed=0x21F1):
    """1 - Champs de bruit colore, 16 s.

    Version 3 (2026-10-23) : retour utilisateur, « les 4/5 ne sont pas
    utilisables : trop faibles ou trop bas ». Mesure de la v2 : apres 2,5 s,
    60 a 99 % de l'energie sous 200 Hz, et un niveau ENTENDU 10 a 20 dB sous le
    debut (le niveleur egalisait le RMS, que le grave remplit sans s'entendre).
    Donc : la couleur ne descend plus sous ~600 Hz de coupure, plus de couche
    brune, une bande medium a la place, un passe-haut 150 Hz (24 dB/oct) sur
    le tout, et un niveleur qui mesure le niveau entendu (`level_flat_heard`).
    La matiere change toujours de couleur, de rugosite et de hauteur au fil
    des secondes, a volume constant.
    """
    r = rng(seed)
    n = int(SR * NOISE_DURATION)
    white = r.standard_normal(n)

    # Couleur : la coupure parcourt 600 Hz -> 15 kHz (25x), plusieurs fois.
    cutoff = 600.0 * (25.0 ** drift(r, n, [0.021, 0.047, 0.083, 0.19, 0.31], [1.0, 0.8, 0.6, 0.35, 0.2]))
    q_lp = np.full(n, 0.7)
    colored = level_flat_heard(
        svf_varying(svf_varying(white, cutoff, q_lp, "lp"), cutoff, q_lp, "lp"), 300.0
    )

    # A la place du brun : une bande medium (300 Hz -> 1,5 kHz), le corps d'une
    # zone sombre qui reste audible.
    mid_cut = 300.0 * (5.0 ** drift(r, n, [0.029, 0.077, 0.013], [1.0, 0.5, 0.7]))
    mid = svf_varying(white, mid_cut, np.full(n, 0.9), "bp")
    mix = drift(r, n, [0.031, 0.071, 0.017], [1.0, 0.5, 0.7])
    body = colored * (1.0 - 0.6 * mix) + level_flat_heard(mid, 300.0) * 0.6 * mix

    # Air : une bande haute qui respire par-dessus.
    air = onepole_lp_varying(white, np.full(n, 9000.0)) - onepole_lp_varying(
        white, np.full(n, 3500.0)
    )
    body += 0.7 * level_flat_heard(air, 300.0) * drift(r, n, [0.017, 0.053, 0.13], [1.0, 0.6, 0.3]) ** 3

    # Bande resonante qui balaie (300 Hz -> 12 kHz), Q mobile.
    band = svf_varying(
        white * 0.4,
        300.0 * (40.0 ** drift(r, n, [0.023, 0.061, 0.14, 0.27], [1.0, 0.7, 0.45, 0.25])),
        2.5 + 18.0 * drift(r, n, [0.019, 0.11, 0.05], [1.0, 0.5, 0.6]),
    )
    body += 0.9 * level_flat_heard(band, 300.0) * drift(r, n, [0.015, 0.049, 0.09], [1.0, 0.6, 0.4]) ** 2

    # Tuyau : comb a delai variable, par zones (0,4 a 6 ms : 170 Hz a 2,5 kHz).
    pipe_zone = drift(r, n, [0.011, 0.037, 0.073], [1.0, 0.6, 0.35]) ** 3
    delay_ms = 0.4 * (15.0 ** drift(r, n, [0.027, 0.069, 0.16], [1.0, 0.5, 0.3]))
    pipe = level_flat_heard(comb_varying(body * 0.5, delay_ms, 0.9 * pipe_zone), 300.0)
    body = body * (1.0 - 0.7 * pipe_zone) + pipe * pipe_zone

    # Rugosite : modulation d'amplitude a 18-90 Hz, par zones.
    am_rate = 18.0 * (5.0 ** drift(r, n, [0.031, 0.087], [1.0, 0.5]))
    am_phase = 2 * np.pi * np.cumsum(am_rate) / SR
    rough_zone = drift(r, n, [0.013, 0.041, 0.097], [1.0, 0.7, 0.4]) ** 3
    body *= 1.0 - 0.85 * rough_zone * (0.5 + 0.5 * np.sin(am_phase))

    # Pas de grave inutile, puis le niveau entendu tenu constant.
    body = highpass(body, 150.0, poles=4)
    return fade_edges(normalize(level_flat_heard(body)))


def metal(seed=0x21F2):
    """2 — Résonances métalliques.

    Dix-huit partiels inharmoniques (loi de plaque, ratios irrationnels) dont
    la fréquence et l'amplitude dérivent lentement : les partiels voisins
    battent entre eux, ce qui donne le chatoiement métallique.
    """
    r = rng(seed)
    t = np.arange(N) / SR
    out = np.zeros(N)

    # La hauteur de base descend puis remonte d'une octave et demie sur la
    # duree : le debut et la fin du fichier ne sonnent pas le meme instrument.
    base_drift = 0.55 + 1.9 * drift(r, N, [0.021, 0.057], [1.0, 0.45])
    base = 92.0
    for k in range(24):
        # Ratios inharmoniques : racine d'entiers, décalés — jamais un multiple
        # entier, sinon on entend une note et non du métal.
        ratio = np.sqrt(1.0 + 3.4 * k) * (1.0 + 0.013 * r.standard_normal())
        freq = base * ratio
        if freq > SR * 0.45:
            continue
        # Dérive de hauteur très lente : c'est elle qui crée les battements.
        vib = 1.0 + 0.0035 * np.sin(2 * np.pi * r.uniform(0.03, 0.31) * t + r.uniform(0, 6.28))
        phase = 2 * np.pi * np.cumsum(freq * vib * base_drift) / SR
        if freq * 2.6 > SR * 0.45:
            continue
        # Chaque partiel a son propre cycle de presence : les familles de
        # partiels entrent et sortent, le timbre se recompose en continu.
        amp = (0.9 / (1.0 + 0.3 * k)) * drift(
            r, N, [r.uniform(0.008, 0.05), r.uniform(0.08, 0.35)], [1.0, 0.5]
        )
        out += amp * np.sin(phase)

    # Un souffle de bruit passé en passe-bande suiveur : le « grain » du métal.
    bp = svf_varying(
        r.standard_normal(N) * 0.5,
        1400.0 * (9.0 ** drift(r, N, [0.023, 0.083], [1.0, 0.5])),
        3.0 + 16.0 * drift(r, N, [0.031], [1.0]),
    )
    out += 1.2 * bp
    return fade_edges(normalize(out))


def crackle(seed=0x21F3):
    """3 — Crépitements et impulsions.

    Grains épars (loi de Poisson, densité modulée), chacun avec sa propre
    hauteur de résonance et sa propre décroissance, sur un lit de statique.
    Tomber sur un grain donne un clic sec, tomber entre deux donne un souffle —
    c'est la texture où l'offset change le plus radicalement le résultat.
    """
    r = rng(seed)
    out = np.zeros(N + SR)

    density = drift(r, N, [0.04, 0.13, 0.021], [1.0, 0.7, 0.5])
    # La hauteur des grains suit une zone qui derive : le debut du fichier
    # crepite grave, la fin siffle. Tirer uniformement partout donnait un
    # spectre moyen identique d'un bout a l'autre, et deplacer l'offset ne
    # s'entendait plus - mesure a l'appui.
    pitch_zone = 120.0 * (60.0 ** drift(r, N, [0.027, 0.073, 0.015], [1.0, 0.6, 0.8]))
    pos = 0
    while pos < N:
        # Intervalle moyen entre grains : de 1,6 ms (crépitement dense) à 22 ms.
        # Le plancher compte : sous ~45 grains/s, les trous dépassent la durée
        # d'une tranche et certains offsets donneraient un coup muet.
        rate = 45.0 + 580.0 * density[pos]
        pos += max(1, int(r.exponential(SR / rate)))
        if pos >= N:
            break
        # Rafale occasionnelle : une poignee de grains colles, qui donne un
        # roulement la ou le reste du fichier donne des clics isoles.
        if r.random() < 0.01:
            pos += int(r.uniform(0.0004, 0.002) * SR)
        length = int(r.uniform(0.003, 0.16) * SR)
        env = np.exp(-np.linspace(0.0, r.uniform(2.0, 20.0), length))
        freq = (pitch_zone[pos] * np.exp(r.uniform(-0.9, 0.9))).clip(60.0, 15000.0)
        # Le grain reste majoritairement TONAL : une part de bruit trop grande
        # rendait chaque grain large bande, et la zone de hauteur ci-dessus
        # devenait inaudible (mesure : ecart de centroide x1,2 seulement).
        tone_mix = r.uniform(0.6, 1.0)
        grain = np.sin(2 * np.pi * freq * np.arange(length) / SR) * tone_mix
        grain += r.standard_normal(length) * (1.0 - tone_mix) * 0.5
        out[pos : pos + length] += grain * env * r.uniform(0.25, 1.0)

    out = out[:N]
    # Le lit de statique suit lui aussi la zone de hauteur, sinon il ramene
    # partout le meme spectre moyen.
    static = onepole_lp_varying(r.standard_normal(N) * 0.07, pitch_zone * 1.8)
    return fade_edges(normalize(out + static))


def sweep(seed=0x21F4):
    """4 — Balayages modulaires.

    Une dent de scie à hauteur lentement modulée en FM, passée dans un
    passe-bande très résonant dont la fréquence balaie tout le spectre. La plus
    « FX » des quatre : matière à risers, transitions et zaps.
    """
    r = rng(seed)
    t = np.arange(N) / SR

    # Porteuse : saw dont la hauteur est modulée par deux oscillateurs lents.
    carrier_hz = 42.0 * (14.0 ** drift(r, N, [0.019, 0.061, 0.14], [1.0, 0.6, 0.3]))
    fm = 1.0 + 0.22 * np.sin(2 * np.pi * 0.9 * t) * drift(r, N, [0.06], [1.0])
    phase = np.cumsum(carrier_hz * fm) / SR
    saw = 2.0 * (phase - np.floor(phase)) - 1.0
    saw += 0.35 * r.standard_normal(N)  # un peu de bruit pour nourrir le filtre

    # Plusieurs allers-retours plutot qu'une montee unique : le fichier
    # traverse quatre ou cinq fois tout le spectre, donc les zones utiles sont
    # reparties partout au lieu d'etre groupees a la fin.
    cutoff = 70.0 * (260.0 ** drift(r, N, [0.21, 0.083, 0.037, 0.013], [1.0, 0.7, 0.5, 0.35]))
    q = 2.5 + 26.0 * drift(r, N, [0.047, 0.19, 0.017], [1.0, 0.5, 0.7])
    body = svf_varying(saw, cutoff, q)

    # Un second balayage, plus lent et plus large, mélangé dessous.
    body += 0.5 * svf_varying(saw, cutoff * 0.33, np.full(N, 2.0))
    return fade_edges(normalize(body))


PAD_DURATION = 16.0


def pad(seed=0x21F5):
    """5 — Pad évolutif lent, atonal ([291]), 16 s.

    Trente-deux partiels dont les hauteurs sont tirées au hasard sur une échelle
    logarithmique (aucun rapport entier : pas de note, pas d'accord) ; chacun
    enfle et s'efface sur sa propre période de 4 à 25 s et dérive de quelques
    centièmes de ton. Dessous, un souffle filtré par un passe-bande très lent.
    Le tout traverse un passe-bas qui s'ouvre et se ferme sur plusieurs secondes :
    déplacer l'offset change la couleur, jamais la tonalité (il n'y en a pas).
    """
    r = rng(seed)
    n = int(SR * PAD_DURATION)
    t = np.arange(n) / SR
    body = np.zeros(n)
    partials = 32
    # Hauteurs : log-uniformes entre 70 Hz et 3,5 kHz, sans rapport simple.
    freqs = np.exp(r.uniform(np.log(70.0), np.log(3500.0), partials))
    for hz in freqs:
        vib_hz = r.uniform(0.04, 0.3)
        vib = 1.0 + 0.004 * np.sin(2 * np.pi * vib_hz * t + r.uniform(0, 2 * np.pi))
        phase = np.cumsum(hz * vib) / SR + r.uniform(0, 1)
        swell_hz = r.uniform(1.0 / 25.0, 1.0 / 4.0)
        swell = 0.12 + 0.88 * (0.5 + 0.5 * np.sin(2 * np.pi * swell_hz * t + r.uniform(0, 2 * np.pi)))
        body += swell * np.sin(2 * np.pi * phase) / np.sqrt(hz)

    # Souffle sous les partiels, passe-bande lent.
    hiss = r.standard_normal(n)
    band = svf_varying(
        hiss,
        180.0 * (14.0 ** drift(r, n, [0.021, 0.053], [1.0, 0.6])),
        np.full(n, 3.0),
    )
    body = body / np.max(np.abs(body)) + 0.18 * band / max(np.max(np.abs(band)), 1e-9)

    # Passe-bas qui respire sur plusieurs secondes.
    body = svf_varying(
        body,
        500.0 * (9.0 ** drift(r, n, [0.031, 0.077], [1.0, 0.5])),
        np.full(n, 0.9),
        out="lp",
    )
    return fade_edges(normalize(level_flat(body)))


def main(only=None):
    print(f"Textures Rift [221] - {SR} Hz, mono PCM 16 bits")
    for name, fn in [
        ("noise-field.wav", noise_field),
        ("metal.wav", metal),
        ("crackle.wav", crackle),
        ("sweep.wav", sweep),
        ("pad.wav", pad),
    ]:
        if only and not any(name.startswith(o) for o in only):
            continue
        write_wav(name, fn())
    print(f"\nEcrit dans {os.path.normpath(OUT_DIR)}")


if __name__ == "__main__":
    import sys

    # `python tools/gen_textures.py noise` ne regenere que le champ de bruit.
    main(sys.argv[1:] or None)
