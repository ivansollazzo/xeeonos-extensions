# Applicazioni XeeonOS (.xapp): specifiche

Una `.xapp` è **un'applicazione in un solo file**, come una `.app` del Mac:

- la metti in `~/Applications` e compare nel menu;
- la cancelli e sparisce dal menu (i suoi dati restano);
- la sostituisci con una versione nuova e si aggiorna;
- doppio clic per aprirla, anche da Scaricati.

Non ci sono store, repository o root. Ogni applicazione gira nella sua
**sandbox**, con i **permessi** che l'utente le dà, ed è **firmata** dal suo
autore. È del tutto separata da Flatpak: Flatpak e AppImage restano
supportati, ma per conto loro.

Il supporto sta nell'estensione opzionale `xapp` di questo repository
(`extensions/xapp/`, si attiva con `sudo xctl extensions enable xapp`): il
programma `xapp` (sorgente in `extensions/xapp/source/`, in Rust),
il monitor di `~/Applications`, il tipo di file e le miniature. Quando
funzionerà bene diventerà parte del sistema. Le ricette
delle applicazioni ufficiali stanno nel repository
[xeeonos-applications](https://github.com/ivansollazzo/xeeonos-applications).

## Indice

1. [Il modello: ogni app ha il suo mini-Arch](#1-il-modello-ogni-app-ha-il-suo-mini-arch)
2. [Il file .xapp](#2-il-file-xapp)
3. [Il manifest](#3-il-manifest)
4. [Firma e fiducia (il Gatekeeper di XeeonOS)](#4-firma-e-fiducia-il-gatekeeper-di-xeeonos)
5. [Dove stanno le cose](#5-dove-stanno-le-cose)
6. [Il monitor e il desktop](#6-il-monitor-e-il-desktop)
7. [L'avvio](#7-lavvio)
8. [La sandbox](#8-la-sandbox)
   - [8bis. Il portal del desktop](#8bis-il-portal-del-desktop-il-tipo-di-applicazione-xapp)
9. [Il contratto con il sistema](#9-il-contratto-con-il-sistema)
10. [I permessi](#10-i-permessi)
11. [La grafica (GPU)](#11-la-grafica-gpu)
12. [I dialoghi](#12-i-dialoghi)
13. [Aggiornamenti delle applicazioni](#13-aggiornamenti-delle-applicazioni)
14. [Costruire una .xapp (xapp build, xapp create)](#14-costruire-una-xapp-xapp-build-xapp-create)
15. [Il comando xapp](#15-il-comando-xapp)
16. [Il codice](#16-il-codice)
17. [Test](#17-test)
18. [Limiti noti e lavori futuri](#18-limiti-noti-e-lavori-futuri)

---

## 1. Il modello: ogni app ha il suo mini-Arch

Sul Mac un'app usa i framework del sistema, che sono stabili. Su Linux il
sistema cambia a ogni snapshot di Arch, quindi una `.xapp` porta con sé il
proprio **sottosistema minimo**: un Arch ridotto all'osso, con solo i
pacchetti che servono a quell'applicazione, preso da uno snapshot
dell'[Arch Linux Archive](https://archive.archlinux.org) scelto da chi la
costruisce (anche vecchio).

- XeeonOS può aggiornarsi senza rompere le applicazioni: ognuna vede sempre
  il proprio mini-Arch.
- Non c'è un runtime condiviso da mantenere: chi pubblica l'app ne sceglie lo
  snapshot e ne è responsabile.
- Il sistema presta solo poche cose, elencate nel
  [contratto con il sistema](#9-il-contratto-con-il-sistema), più i driver
  grafici (sezione 11).

Lo stesso schema lo usa lo Steam Linux Runtime (pressure-vessel).

Prezzi accettati: un'app grande pesa 150-400 MB (le librerie sono ripetute
in ogni app), e un'app costruita su uno snapshot vecchio si tiene le
librerie di allora (`xapp info` avvisa oltre i 12 mesi).

## 2. Il file .xapp

```
  +-------------------------------+  offset 0
  | immagine erofs (sola lettura) |  il mini-Arch: /usr, /etc, /xapp/...
  | con il "magic" xapp nei primi |  (i primi 1024 byte, che erofs non usa)
  | byte                          |
  +-------------------------------+  dimensione dell'immagine
  | manifest.json                 |  cos'è l'applicazione (sezione 3)
  | firma (64 byte, Ed25519)      |  dei byte grezzi del manifest
  | icona (PNG o SVG)             |
  | trailer (32 byte)             |  dove stanno i pezzi
  +-------------------------------+  fine del file
```

- **Immagine erofs**: la stessa tecnologia di `/usr` e delle estensioni di
  XeeonOS. Si monta e si usa subito, senza estrazione né spazio doppio (come
  un `.dmg` aperto al volo), ed è in sola lettura per costruzione:
  l'applicazione non può modificarsi. Compressione lz4hc.
- **Magic**: `\x7fXAPP1\n\0` all'offset 0. Grazie a questo, il desktop
  riconosce una `.xapp` dal contenuto (`application/x-xapp`) anche se il
  file viene rinominato.
- **Trailer** (ultimi 32 byte): `XAPPEND1`, dimensione dell'immagine (u64),
  lunghezza del manifest (u32), lunghezza dell'icona (u32), 8 byte a zero.
  Tutti i numeri sono little-endian. Manifest, firma e icona si leggono
  dalla fine del file senza montare nulla.
- Limiti: manifest al massimo 1 MiB, icona al massimo 4 MiB.

Dentro l'immagine, `/xapp/packages.txt` elenca i pacchetti Arch installati.

## 3. Il manifest

JSON, scritto da `xapp build` e firmato dall'autore. Il formato è il
**formato 1**: tutti i campi previsti esistono già, così le versioni
successive di xapp aggiungono funzioni senza cambiare il formato.

| Campo | Significato |
|---|---|
| `format` | `1` |
| `app_id` | id in stile dominio inverso (`org.mozilla.firefox`): lettere, cifre, `.`, `_`, `-`, almeno due parti |
| `name`, `names` | nome, e nomi tradotti (`{"it": "..."}`) |
| `description` | descrizione breve |
| `version` | versione confrontabile (`146.0.1`) |
| `exec` | comando (percorso assoluto nell'immagine) e argomenti fissi |
| `arch` | `x86_64` |
| `snapshot` | snapshot di Arch dell'immagine (`2026-11-13`) |
| `host_contract` | versione del [contratto con il sistema](#9-il-contratto-con-il-sistema) che serve (`1`) |
| `gpu` | `host` (driver del sistema, predefinito) o `bundled` (driver propri) |
| `user_namespaces` | l'app fa una sandbox propria dentro la nostra (Chromium, Electron, WebKitGTK, Qt WebEngine): deciso da `xapp build`, sezione 8 |
| `glibc`, `libstdcxx` | versioni di glibc e gcc-libs dell'immagine |
| `capabilities` | quello che serve all'app per funzionare (oggi solo `x11`): deciso da `xapp build`, mai chiesto, sezione 10 |
| `permissions` | i permessi che l'app **può** chiedere, ognuno con il motivo: `[{"id": "camera", "reason": "Per le videochiamate"}]` |
| `mime_types` | tipi di file che apre |
| `categories` | categorie del menu |
| `wm_class` | nome delle sue finestre (Wayland app id), per la dock |
| `icon_type`, `icon` | `png` o `svg`, e `{size, sha256}` dell'icona |
| `image` | `{size, sha256}` dell'immagine |
| `signer` | chiave pubblica Ed25519 dell'autore (64 cifre esadecimali) |

Un manifest di formato diverso, con un contratto più nuovo di quello del
sistema o con campi obbligatori sbagliati viene rifiutato.

## 4. Firma e fiducia (il Gatekeeper di XeeonOS)

- L'autore firma con Ed25519 i **byte grezzi** del manifest (niente JSON
  "normalizzato", quindi nessuna ambiguità).
- Il manifest contiene lo SHA-256 dell'immagine e dell'icona: la firma di
  pochi byte copre tutto il file. Un solo byte cambiato e l'app viene
  **rifiutata**, senza modo di forzarla.
- **Pinning**: la prima chiave vista per un `app_id` viene ricordata
  (`~/.local/state/xapps/keys/<id>`). Un file con lo stesso id e un'altra
  chiave viene rifiutato: potrebbe essere un falso aggiornamento.
- **Revoca**: `/usr/lib/xapp/revoked` (arriva con gli aggiornamenti di
  XeeonOS, quindi è firmata come loro) e `/etc/xapp/revoked`
  (dell'amministratore). Righe `key <chiave>`, `app <id>`,
  `image <sha256>`. Un'app revocata non si apre.
- **Autori fidati**: chiavi in `/usr/lib/xapp/trusted.d/*.pub` (con il
  sistema) e `/etc/xapp/trusted.d/*.pub` (amministratore). Le loro app si
  aprono subito.
- **Autore sconosciuto**: file firmato e integro, ma da una chiave che
  nessuno garantisce. Alla prima apertura c'è la domanda "Apri comunque",
  e la risposta viene ricordata per quella chiave
  (`~/.local/state/xapps/approved/<id>`).

Il controllo completo dell'immagine (circa un secondo per un'app grande)
viene ricordato per file, dimensione, data, inode e digest
(`~/.local/state/xapps/verified`), così l'avvio successivo non rilegge
tutto. `xapp verify` lo rifà sempre per intero.

## 5. Dove stanno le cose

Tutto è dell'utente, senza root:

| Percorso | Cosa |
|---|---|
| `~/Applications/` | i file `.xapp` (cartella vera, una per utente) |
| `~/.var/xapps/<id>/` | i dati dell'app, cioè la sua home nella sandbox. **Mai cancellati da soli**: rimettere il file riporta l'app com'era |
| `~/.local/state/xapps/` | registro (`apps/`), chiavi (`keys/`, `approved/`), permessi (`permissions/`), icone, versioni usate, cache dei controlli |
| `~/.local/share/applications/<id>.desktop` | voci di menu |
| `~/.config/autostart/<id>.desktop` | avvio al login (permesso `autostart`) |
| `$XDG_RUNTIME_DIR/xapp/` | immagini montate (`mnt/`), istanze in esecuzione (`run/`), dialoghi |

Del sistema (dentro l'estensione): `/usr/bin/xapp`,
`/usr/lib/systemd/user/xapp-sync.*`, `/usr/lib/xapp/` (chiavi fidate,
revoche), `/usr/share/xapp/xeeonos-xapp.xml` (tipo di file),
`/usr/share/applications/xapp-run.desktop` (doppio clic),
`/usr/share/thumbnailers/xapp.thumbnailer` (miniature in File).

## 6. Il monitor e il desktop

Il monitor è un servizio **utente** (systemd `--user`, nessun root):

- `xapp-sync.path` sorveglia `~/Applications` (e la crea se manca);
- `xapp-sync.service` esegue `xapp sync`: all'avvio della sessione e a ogni
  cambiamento della cartella;
- `xapp-sync.timer` ripete il controllo completo ogni 15 minuti, per quello
  che la sorveglianza può perdere.

`xapp sync`:

1. legge ogni `.xapp` della cartella. Un file ancora in copia è incompleto:
   viene riprovato dopo poco, fino a circa un minuto;
2. controlla firma, revoca, pinning e immagine (sezione 4);
3. se più file hanno lo stesso id, vince la versione più alta;
4. per ogni app scrive la voce di menu `~/.local/share/applications/<id>.desktop`
   (con `Exec=xapp run <id> %U`), l'icona e i tipi di file, e ricorda la
   chiave (pinning);
5. toglie le voci delle app il cui file non c'è più. I dati restano.

Il tipo di file `application/x-xapp` (per nome `*.xapp` e per magic) viene
copiato nel database dell'utente (`~/.local/share/mime`), perché quello del
sistema dentro `/usr` sigillato non si può ricostruire. Il gestore
predefinito è `xapp-run.desktop` (`xapp run %f`), aggiunto a
`~/.config/mimeapps.list` solo se l'utente non ha già scelto altro. Così il
**doppio clic** su una `.xapp` la apre. File mostra come miniatura l'icona
dell'app (`xapp thumbnail`).

Ogni file scritto da xapp porta `X-Xapp-Id=`: xapp cancella solo i propri.

## 7. L'avvio

`xapp run <file|id> [argomenti]` (anche dal menu o con un doppio clic):

1. **Controllo del file**: firma, revoca, pinning, immagine. Se fallisce,
   l'app viene rifiutata con un messaggio (in una finestra di dialogo se
   lanciata dal menu).
2. **Domande prima di aprire**, come sul Mac:
   - file fuori da `~/Applications`: "Apri solo questa volta" o "Sposta in
     Applicazioni e apri" (preselezionata), oppure "Annulla";
   - autore sconosciuto: "Apri comunque" o "Annulla" (preselezionata);
   - versione più vecchia di quella usata prima: conferma.
3. **Permessi** dichiarati e non ancora decisi, uno per uno, con il motivo
   scritto dall'app. Per ora le domande arrivano **all'avvio**.
4. **Sandbox**: monta l'immagine, prepara l'ambiente, avvia l'app in uno
   scope systemd tutto suo (`app-xapp-<id>-<pid>.scope`). Lo scope dà i
   limiti di risorse (TasksMax), il nome con cui il portal del desktop
   riconosce l'app (sezione 8bis), e i log.

## 8. La sandbox

È fatta con **bubblewrap** (la stessa base di Flatpak), senza podman,
distrobox o container. L'app parte in nuovi namespace (utente, mount, PID,
IPC, UTS, cgroup, e rete se non è permessa), su una radice vuota riempita
pezzo per pezzo:

| Nella sandbox | Da dove |
|---|---|
| `/usr`, `/opt`, ... e i link `/bin`, `/lib` | l'immagine dell'app, in sola lettura (montata con erofsfuse in `$XDG_RUNTIME_DIR/xapp/mnt/<digest>`) |
| `/etc` | quello dell'immagine, più quello che presta il sistema (sezione 9) e un `passwd`/`group` con il solo utente |
| `/proc`, `/dev`, `/sys` (solo dispositivi, sola lettura) | nuovi |
| `/tmp`, `/var`, `/run` | vuoti, privati |
| `$HOME` | `~/.var/xapps/<id>` (i dati dell'app), lettura e scrittura |
| `$HOME/Documenti`, ... | solo le cartelle permesse, allo stesso percorso |
| `/run/user/<uid>` | Wayland, PipeWire (se permesso), il bus di sessione filtrato, `doc/`: i file dati dal portal (selettore dei file) |
| `/.xapp-info` | chi è l'app, per il portal del desktop (sezione 8bis) |
| `/run/host` | font, icone, temi, driver grafici del sistema |
| `/run/xapp/docs/<n>/` | i file che l'app è stata chiamata ad aprire |

Altre protezioni:

- **seccomp**: un filtro BPF scritto a mano (`xapp-sandbox/src/seccomp.rs`)
  rifiuta ptrace, mount, moduli del kernel, kexec, keyring, BPF, perf,
  unshare, setns, clone con `CLONE_NEWUSER`, cambio dell'ora, swap, reboot,
  `TIOCSTI` e altro. clone3 risponde "non supportato", così glibc ripiega su
  clone, di cui il filtro vede i flag. I programmi x32 vengono fermati; quelli
  i386 anche, a meno che l'app abbia programmi a 32 bit (`multilib` nel
  manifest: dalla ricetta, o perché l'immagine ha un loader a 32 bit). Allora
  le loro chiamate passano per le stesse regole, con i numeri i386, come fa
  il multiarch di Flatpak. Senza questo Steam, Wine e ogni programma a 32
  bit morivano con SIGSYS.
- **Sandbox propria dentro la nostra** (`user_namespaces` nel manifest):
  Chromium, Electron, CEF, WebKitGTK e Qt WebEngine isolano le proprie
  pagine con i namespace utente, e senza non partono. Non è un permesso e
  l'utente non sceglie niente: lo decide `xapp build` guardando cosa c'è
  nell'immagine (`chrome-sandbox`, `v8_context_snapshot.bin`, `libcef.so`,
  `QtWebEngineProcess`, `WebKitWebProcess`), e la ricetta può forzarlo con
  `"user_namespaces": true|false`. Per queste app il filtro permette
  unshare, clone con `CLONE_NEWUSER` e i mount: i mount richiedono
  `CAP_SYS_ADMIN`, che l'app ha solo nei namespace che crea lei (la
  sandbox toglie ogni capability), quindi può cambiare solo la sua
  sandbox, mai la nostra. setns e il resto restano rifiutati, e i
  namespace nuovi ereditano il filtro. Solo dentro lo scope systemd. Le
  altre app (Firefox compreso) non ne hanno bisogno: glycin, il caricatore
  di immagini di GTK, se non ha i namespace carica le immagini senza la
  sua sandbox, sempre dentro la nostra.
- `--cap-drop ALL`, `--die-with-parent`.
- **Bus di sessione filtrato** con `xdg-dbus-proxy`, politica "nega tutto".
  Passano solo il proprio nome e il suo nome di lettore multimediale
  (`org.mpris.MediaPlayer2.<ultima parte dell'id>`: tasti multimediali e
  controlli del desktop; VLC senza si chiude credendo di essere già
  aperto), l'icona nella barra di sistema (`org.kde.StatusNotifierWatcher`),
  il portal del desktop per le parti che chiedono all'utente, aprono
  soltanto o dicono com'è il sistema (link e file, selettore dei file,
  impostazioni come il tema scuro, inibizione della sospensione,
  condivisione dello schermo, stampa, e-mail, il proprio segreto, scorciatoie
  globali, cestino, stato della rete, proxy, memoria, risparmio energetico),
  il document portal, le notifiche e il portal Camera (con i rispettivi
  permessi), più i nomi aggiunti a mano con `xapp doctor` (sezione 10).
  Altre app, GNOME Shell, il portachiavi dell'utente e il file manager non
  esistono per l'app.
- **Impostazioni di GNOME**: dconf non c'è nella sandbox, quindi GTK le
  legge dal portal (`GTK_USE_PORTAL=1` per GTK 3, `GDK_DEBUG=portals` per
  GTK 4): pulsanti della finestra (minimizza, massimizza, chiudi), tema,
  caratteri, stile scuro. Così passa anche il selettore dei file.
- Ambiente ripulito (`--clearenv`): passano solo lingua, terminale, desktop
  e fuso orario.
- **Niente X11**, tranne per le app che hanno la capacità `x11`
  (sezione 10).

**File aperti**: un file passato all'app (doppio clic su un documento, o
argomento) viene prestato da solo in `/run/xapp/docs/<n>/<nome>`, anche se
la sua cartella non è permessa. Aprirlo è il consenso dell'utente, come il
"powerbox" del Mac. I file dentro una cartella permessa mantengono il loro
percorso.

Cosa la sandbox **non** protegge (detto chiaramente): la GPU e la tastiera
sono condivise con il resto della sessione. Su GNOME il socket Wayland è
quello normale (Mutter non dà comunque protocolli privilegiati ai client
normali). Un'app con la capacità `x11` può vedere le altre finestre X11.

## 8bis. Il portal del desktop: il tipo di applicazione "xapp"

xdg-desktop-portal riconosce le app Flatpak, Snap e Linyaps; le altre le
tratta come programmi del sistema, senza limiti. L'estensione porta una
versione del portal con un quarto tipo, **"xapp"**
(`extensions/xapp/portal/xapp-app-info.patch`, da proporre upstream):
`xdg-desktop-portal` e `xdg-document-portal` della stessa versione dello
snapshot di Arch, compilati in `build.sh` con le stesse funzioni del
pacchetto di Arch e messi in `/usr/lib` al posto di quelli del sistema
(se lo snapshot cambia versione, la build si ferma finché la patch non è
aggiornata in `portal/source`).

- **Chi è**: lo scope `app-xapp-<id>-<n>.scope`, da cui l'app non può
  uscire, e il file `/.xapp-info` nella sua radice (in sola lettura):

  ```ini
  [Application]
  Id=org.mozilla.firefox

  [Instance]
  Id=<pid di xapp>

  [Context]
  Network=shared|none
  ```

  Se c'è lo scope, comanda lo scope: un'app con `user_namespaces` può
  cambiare radice (chroot) e far sparire o falsificare `/.xapp-info`, non
  lo scope. Per questo il tipo "xapp" viene controllato **per primo**,
  prima di Flatpak.
- **Il bus**: il portal vede `xdg-dbus-proxy`, non l'app. Il proxy gira in
  una piccola sandbox sua, con lo stesso `/.xapp-info` (come fa Flatpak).
- **I file**: il selettore dei file registra i file scelti nel document
  portal; l'app li vede in `$XDG_RUNTIME_DIR/doc/<id>/<nome>` (la sandbox
  monta lì `doc/by-app/<id>`, solo la sua parte). `xapp run` avvia il
  document portal se non è ancora partito.
- I permessi decisi nei dialoghi del portal (posizione, screenshot, ...)
  restano legati all'id dell'app, come per Flatpak.

Il portal nuovo vale dal login successivo all'attivazione dell'estensione
(quello già in esecuzione resta il vecchio).

## 9. Il contratto con il sistema

È l'elenco esatto di quello che XeeonOS mette dentro ogni sandbox. Tutto il
resto viene dall'immagine dell'app. **Versione 1**:

- sempre: socket Wayland, bus di sessione filtrato, `/.xapp-info`, i file
  dati dal document portal (`$XDG_RUNTIME_DIR/doc`), `/etc/hosts`,
  `/etc/hostname`, `/etc/machine-id`, fuso orario (`/etc/localtime`),
  certificati CA (`/etc/ssl`, `/etc/ca-certificates`), font del sistema e
  dell'utente (`/run/host/fonts`, via `FONTCONFIG_FILE`), icone e temi
  (`/run/host/share`, in `XDG_DATA_DIRS`), le lingue del sistema
  (`/usr/lib/locale`, se l'immagine ne ha la cartella), lingua e terminale;
- con il permesso `network`: la rete e il DNS (`/etc/resolv.conf`);
- con `gpu`: `/dev/dri` e, se l'app vuole `gpu: host`, lo stack grafico del
  sistema (sezione 11);
- con `audio`: PipeWire e PulseAudio; con `camera`: `/dev/video*`,
  `/dev/media*` e PipeWire;
- con le cartelle: le cartelle permesse.

Se il contratto cambia, cambia il numero (`host_contract` nel manifest):
un'app che chiede un contratto più nuovo di quello del sistema non si
apre.

## 10. I permessi

Sono come il TCC del Mac. L'app dichiara nel manifest i permessi che **può**
chiedere, ognuno con il motivo. Un permesso non dichiarato l'app non può
chiederlo: può darglielo solo l'utente, a mano (vedi "L'ultima parola è
dell'utente").

| Permesso | Cosa dà | Di default |
|---|---|---|
| `network` | rete e internet | sì |
| `gpu` | accelerazione grafica | sì |
| `notifications` | notifiche | sì |
| `documents`, `downloads`, `pictures`, `music`, `videos`, `desktop` | la cartella utente corrispondente (nomi tradotti da `~/.config/user-dirs.dirs`) | chiesto |
| `audio` | suono e microfono (in v1 insieme: il socket PipeWire non li separa bene) | chiesto |
| `camera` | fotocamera | chiesto |
| `autostart` | avvio al login | chiesto |

**Permessi personalizzati**, come quelli di Flatpak, per quello che il
catalogo non ha:

| Permesso | Cosa dà |
|---|---|
| `dbus-talk:<nome>` | parlare con un nome del bus di sessione (come `--talk-name`) |
| `dbus-own:<nome>` | prendere un nome del bus (come `--own-name`) |
| `folder:~/<cartella>[:ro]`, `folder:/mnt/...` | una cartella precisa, anche in sola lettura (nella home, ma non la home intera né una cartella nascosta come `~/.ssh`; oppure sotto `/mnt`, `/media`, `/run/media`) |
| `device:input` | controller e joystick (`/dev/input`, `hidraw`, `uinput`) |
| `device:all` | tutti i dispositivi di `/dev` |

A differenza di Flatpak non vengono dati in silenzio: hanno il loro motivo
e sono **chiesti all'utente** come gli altri, mai dati di default. Quelli
delicati (portachiavi, systemd, Shell, KWin, ..., e `device:all`) sono
segnati "attenzione" e `--yes` non li dà.

I nomi del bus possono finire con `.*` (`dbus-own:com.example.Gioco.*`):
il nome e tutti quelli sotto, come `--own=...*` di xdg-dbus-proxy. Serve
alle app che prendono un nome con un numero diverso a ogni avvio (Steam:
`...LaunchAlongsideSteam.Instance394`). Il prefisso deve avere almeno tre
parti (mai `org.kde.*`), ed è delicato se copre un nome delicato.
`xapp doctor`, per un nome rifiutato che finisce con un numero, propone
direttamente il prefisso.

**Alla prima apertura** i permessi in sospeso sono chiesti tutti insieme,
in una lista a caselle (come tasksel): niente spuntato, l'utente spunta
quello che permette (spazio) e conferma.

"Di default sì" vale per le app di un autore fidato o approvato, se l'utente
non ha detto di no. Le scelte stanno in
`~/.local/state/xapps/permissions/<id>` (righe `<permesso>=yes|no`) e si
cambiano con `xapp permissions <app> <permesso> yes|no|ask`. Quello che non
è permesso **non esiste** nella sandbox: una cartella negata non c'è
proprio. Se all'avvio non c'è nessuno a cui chiedere, i permessi in sospeso
non vengono dati per quella volta (e vengono richiesti la volta dopo).

### L'ultima parola è dell'utente (come flatpak override)

```
xapp permissions <app>                         i permessi e il loro stato
xapp permissions <app> <permesso> yes|no|ask   cambia una scelta
xapp permissions <app> --reset                 dimentica le scelte (richiede)
xapp permissions --all                         le scelte per tutte le app
xapp permissions --all <permesso> yes|no|ask   una scelta per tutte le app
```

- Con `no` si toglie un permesso anche se l'app lo dichiara, e anche se di
  default sarebbe dato (per esempio `network`).
- Con `yes` si dà anche un permesso che l'app **non** dichiara, sia del
  catalogo sia personalizzato (`folder:~/Giochi`, `device:input`, ...).
  `xapp permissions` e `xapp info` lo mostrano con il motivo "added by
  you". Con `ask` (o `no`) lo si toglie.
- `--all` vale per tutte le app (`~/.local/state/xapps/permissions.all`).
  La scelta fatta per una singola app vince su quella per tutte, e tutte e
  due vincono sul manifest.
- I permessi delicati (`device:all`, il portachiavi, ...) si possono dare,
  ma il comando stampa prima un avvertimento.

Anche `xapp doctor` usa questo meccanismo: un permesso voluto dall'app ma non
dichiarato lo propone comunque (aggiunto a mano), e ricorda che la ricetta
dovrebbe dichiararlo.

### Due livelli: capacità e permessi

Non tutto quello che manca a un'app è una questione di privacy. Per non
dover seguire app per app cosa le serve, le cose sono divise in due:

- **Capacità**: quello che serve all'app per funzionare, senza dati
  dell'utente in gioco. Le decide `xapp build` guardando l'immagine, stanno
  nel campo `capabilities` del manifest firmato e **non vengono mai
  chieste**. `xapp info` le mostra ("Serve: ...").
- **Permessi**: quello che tocca la privacy (cartelle, fotocamera,
  microfono, ...), dichiarato con un motivo e chiesto all'utente, come
  sopra.

| Capacità | Cosa dà | Come la decide `xapp build` |
|---|---|---|
| `x11` | finestre X11 (isolamento più debole dalle altre finestre X11) | interfacce che si aprono solo su X11: l'interfaccia Qt di VLC 3, GTK 2, Qt 5/6 senza il plugin Wayland, Java AWT/Swing, Electron/Chromium, libX11 senza libwayland-client |

La ricetta può aggiungerne con `"capabilities": ["x11"]`; un permesso `x11`
scritto in una ricetta o in un manifest vecchio diventa la capacità `x11`.

### Quando manca qualcosa: xapp run --trace e xapp doctor

Se un'app non parte o non fa qualcosa, non si indovina:

```
xapp run --trace <app>     avvia e nota cosa la sandbox rifiuta
xapp doctor <app>          propone di permettere quello che è stato rifiutato,
                           e i pacchetti di quello che mancava
```

Con `--trace` il filtro del bus scrive ogni messaggio (`xdg-dbus-proxy
--log`) e, se l'app non ha X11, al suo posto c'è un finto server X11
(`DISPLAY=:0`) che conta chi bussa. Lo stesso per il suono: senza il
permesso `audio`, un finto PipeWire e un finto PulseAudio. Le chiamate
rifiutate al portal Camera o alle notifiche contano come "voleva la
fotocamera" o "voleva le notifiche". Alla fine il resoconto (codice di
uscita, durata, nomi del bus rifiutati, X11, permessi voluti) va in
`~/.local/state/xapps/trace/<id>` e viene stampato.

Il resoconto nota anche quello che all'app **manca** e che nessun permesso
può dare, perché lo deve dare la ricetta:

- quello che l'app scrive sullo standard error (che resta visibile come
  sempre): `tar: command not found` (anche `sh: 1: zenity: not found`),
  `error while loading shared libraries: libnss3.so`, e gli errori di chi
  prova a farsi una sandbox sua senza `user_namespaces` ("No usable
  sandbox", "setting up uid map", l'helper SUID di Chromium/Electron,
  ...);
- lo stesso nei file di log che l'app scrive durante l'avvio nei suoi
  dati (`*.log`, o i file di una cartella `log`/`logs`): c'è chi scrive
  solo lì (Steam);
- i programmi fermati dal filtro di sistema: `coredumpctl` dopo l'avvio,
  i processi dell'utente morti con `SIGSYS` (per esempio un programma a 32
  bit in un'immagine costruita senza `multilib`).

`xapp run --trace` li stampa a parte ("it missed") dai rifiuti. Il
resoconto viene scritto anche mentre l'app gira (ogni 5 secondi), così
resta anche se la si chiude da fuori (un kill, un timeout).
`xapp doctor` non li chiede: cerca nello snapshot dell'app (lo stesso
database dei file di `pacman -F` che usa il wizard, in un container) i
pacchetti che hanno `usr/bin/<comando>` o `usr/lib{,32}/<libreria>`, li
mostra (quelli per hardware che XeeonOS non guida solo se non c'è altro), e
dice quali aggiungere a `"packages"` nella ricetta prima di ricostruirla;
per le sandbox proprie suggerisce `"user_namespaces": true`, per i
programmi fermati `"multilib"`. Vale per tutte le sorgenti del wizard (Arch,
AUR, URL, cartella): la ricetta ha sempre il suo snapshot e la lista
`packages`.

`xapp doctor` chiede, una cosa per volta, se permettere ciascuna di quelle
cose (e i permessi che l'utente aveva negato). Un permesso voluto ma non
dichiarato nel file viene proposto lo stesso, come aggiunta dell'utente
(sopra). I nomi delicati
(portachiavi, systemd, GNOME Shell, KWin, Flatpak, le implementazioni dei
portal, ...) sono proposti con "no" come risposta predefinita, e `--yes`
non li permette mai. Le aggiunte stanno in
`~/.local/state/xapps/overrides/<id>` (righe `own=`, `talk=`, `see=`,
`capability=`), sono dell'utente e non dell'autore, valgono solo per quella
app e si tolgono con `xapp doctor <app> --reset`. `xapp info` le mostra.
Un nome del bus valido è un nome ben formato, mai `org.freedesktop.DBus`
né un nome unico (`:1.42`).

## 11. La grafica (GPU)

Un mini-Arch vecchio ha una Mesa vecchia, che non conosce le schede video
nuove. Con `gpu: host` (il default) il sistema presta la sua Mesa, sempre la
più recente: OpenGL, EGL, GBM e i driver Vulkan, con tutte le librerie che
caricano (trovate con `ldd`), **glibc e libstdc++ comprese**. Lo fa anche
Steam.

- Funziona perché una glibc nuova fa girare programmi compilati per una
  vecchia: vince sempre la versione più nuova. Prima si controlla che glibc
  e libstdc++ del sistema siano almeno nuove quanto quelle dell'app;
  altrimenti l'app tiene i suoi driver (o disegna senza accelerazione) e
  xapp lo dice.
- Ogni libreria prestata sostituisce il file con lo stesso nome dell'app
  (montata sopra) oppure, se l'app non ce l'ha, va in `/run/host/gpu/lib`
  (in `LD_LIBRARY_PATH`). Il set di glibc arriva sempre tutto dal sistema.
- I driver vanno in `/run/host/gpu` (`LIBGL_DRIVERS_PATH`,
  `GBM_BACKENDS_PATH`, `__EGL_VENDOR_LIBRARY_DIRS`). Le descrizioni dei
  driver Vulkan vengono riscritte con il solo nome della libreria
  (`VK_DRIVER_FILES`).

Con `gpu: bundled` l'app usa i driver della propria immagine.

## 12. I dialoghi

Per ora sono **dialoghi da terminale**, disegnati con `whiptail` in bianco e
nero come quelli di xctl:

1. se `xapp` gira in un terminale, le domande compaiono lì;
2. se l'app parte dal menu o con un doppio clic, si apre una finestra di
   terminale (GNOME Console) con le domande. xapp scrive le domande in
   `$XDG_RUNTIME_DIR/xapp/dialogs/<id>.questions`, la finestra esegue
   `xapp dialog <file>` e scrive le risposte accanto. Se la finestra viene
   chiusa senza rispondere, vale come "no";
3. senza terminale né finestra non c'è risposta: vale come "no".

Variabili: `XAPP_DIALOG=text` (righe di testo semplici invece di whiptail),
`XAPP_TERMINAL=<programma>` (il terminale da aprire, `none` per nessuno),
`--yes` o `XAPP_ASSUME_YES=1` (risponde a tutto con la prima scelta, per
script e CI). Più avanti i dialoghi potranno diventare finestre grafiche
senza cambiare il resto.

## 13. Aggiornamenti delle applicazioni

Si aggiorna **sostituendo il file** in `~/Applications`:

- stesso `app_id` e stessa chiave (pinning) = aggiornamento;
- se ci sono più file, vince la versione più alta;
- una versione più vecchia di quella già usata chiede conferma all'avvio;
- la nuova immagine ha un altro digest e quindi un'altra cartella di
  montaggio: le istanze già aperte continuano con la vecchia finché non si
  chiudono.

L'app non può aggiornarsi da sola (la sua immagine è in sola lettura): gli
aggiornamenti interni, come quello di Firefox, vanno spenti nella ricetta.
Gli aggiornamenti dalla rete non sono previsti per ora.

## 14. Costruire una .xapp (xapp build, xapp create)

Una **ricetta** è una cartella con `recipe.json`:

```json
{
  "app_id": "org.mozilla.firefox",
  "name": "Firefox",
  "names": {"it": "Firefox"},
  "description": "Browser web",
  "snapshot": "2026/11/13",
  "packages": ["firefox"],
  "exec": ["/usr/lib/firefox/firefox"],
  "gpu": "host",
  "permissions": [{"id": "network", "reason": "Per navigare"}],
  "mime_types": ["text/html"],
  "categories": ["Network", "WebBrowser"],
  "wm_class": "firefox",
  "icon": "/usr/share/icons/hicolor/128x128/apps/firefox.png",
  "remove": ["usr/lib/firefox/updater"],
  "script": "build.sh"
}
```

- `version` si può omettere: è la versione del primo pacchetto.
- `icon`: un percorso dentro l'immagine (inizia con `/`) o un file della
  ricetta.
- `script`: un passo della ricetta, eseguito con `ROOT=<radice>` dopo
  l'installazione dei pacchetti (file aggiuntivi, impostazioni).
- `remove`: percorsi da togliere dalla radice.
- `sources`: archivi pubblicati dagli autori dell'app (binari ufficiali),
  `[{"url": "https://...", "sha256": "...", "to": "/opt/<nome>"}]`:
  scaricati, controllati con lo SHA-256 (un file diverso ferma la build) e
  scompattati in `to` (senza la cartella unica che li contiene), dopo i
  pacchetti e prima di `script`. Solo `https://` e solo dentro `/opt`.
  Una sorgente può essere anche una cartella dell'utente,
  `{"dir": "<cartella>", "to": "/opt/<nome>"}`: assoluta, o relativa alla
  ricetta (per tenerla o condividerla insieme ai file). Viene montata in
  sola lettura nel container e copiata allo stesso modo.
- `aur`: pacchetti dell'AUR, `[{"name": "app", "commit": "<40 cifre>"}]`:
  costruiti nel container dal commit del loro repository AUR (quello del
  PKGBUILD letto), sui pacchetti dello snapshot, poi installati con le loro
  dipendenze. Vedi `--aur` più sotto.
- `user_namespaces`: di solito si omette, `xapp build` lo capisce
  dall'immagine (sezione 8).
- `multilib`: `true` aggiunge il repository multilib dello stesso
  snapshot (pacchetti a 32 bit, `lib32-*`, e app che ne hanno bisogno).
  La build fa solo quello che dice la ricetta; è il wizard a proporlo.
  Il manifest lo riporta (anche quando l'immagine porta un suo loader a
  32 bit) e la sandbox lascia girare i programmi i386.

`xapp build <ricetta>`:

1. in un container Arch Linux (podman o docker; `XAPP_CONTAINER` per
   sceglierlo) installa con `pacstrap` i pacchetti **dallo snapshot della
   ricetta**, con i loro hook (cache delle librerie, icone, font, schemi
   GSettings), senza documentazione, header e librerie statiche. Se la
   ricetta lascia a pacman la scelta di un pacchetto virtuale
   (`vulkan-driver`, `ttf-font`, `java-runtime`...), lo dice con un
   avviso: pacman prenderebbe il primo della sua lista (per
   `vulkan-driver`, nvidia-utils). Il wizard lo chiede e lo scrive;
2. toglie quello che serve solo a un sistema intero (`/boot`, `/home`,
   `/var`, ...) e i file che solo il sistema legge: regole udev, moduli
   del kernel, servizi e impostazioni di systemd del sistema, utenti di
   sistema, polkit, D-Bus di sistema (`HOST_ONLY_RE` in `scan.sh`). Dentro
   la sandbox non li usa nessuno (Steam ne portava 491 unità);
3. crea l'immagine erofs, scrive il magic, calcola gli hash, scrive il
   manifest, lo firma e chiude il file;
4. lo installa in `~/Applications` (`--no-install` per non farlo).

La chiave dell'autore: `--key <file>` (creata se manca), oppure il testo
della chiave in `XAPP_SIGNING_KEY`, oppure `~/.config/xapp/signing.key`
(creata la prima volta). **Va conservata**: gli aggiornamenti di un'app
devono avere la stessa chiave. `--root <cartella>` impacchetta una radice
già pronta, senza container.

### Il wizard (xapp create)

Scrivere una ricetta a mano non serve: `xapp create` la propone partendo
da un pacchetto Arch, da un pacchetto dell'AUR, da un archivio pubblicato
dagli autori dell'app o da una cartella sul disco.

```
xapp create --arch gnome-calculator
xapp create --aur xclicker
xapp create --url "https://download.mozilla.org/?product=firefox-latest-ssl&os=linux64&lang=it"
xapp create --dir ~/Progetti/mia-app/dist
```

Con `--arch`:

1. in un container Arch guarda il pacchetto **nello snapshot del sistema**
   (`XEEONOS_ARCH_SNAPSHOT` di `/usr/lib/os-release`, oppure
   `--snapshot`): descrizione, versione, dipendenze, file;
2. dalla voce di menu (`.desktop`) prende nome, nomi tradotti, comando,
   icona (se il pacchetto non ce l'ha, la prende dal tema di icone dello
   snapshot che la contiene e aggiunge quel pacchetto), categorie, tipi di
   file e classe della finestra; l'id è il nome
   del `.desktop` se è già in stile reverse-DNS, altrimenti
   `org.xeeonos.arch.<pacchetto>` (`--id` per sceglierlo);
3. **i permessi**, in una lista a caselle come tasksel: tutti quelli del
   catalogo, con già spuntati (e il perché accanto) quelli che
   probabilmente servono. Li deduce dalle librerie che il pacchetto si
   porta dietro (curl, libsoup, nss, WebKit → rete; Mesa, Vulkan, GTK4,
   Qt → GPU; libnotify, libadwaita → notifiche; PipeWire, PulseAudio,
   ALSA, GStreamer → audio; libcamera, v4l → fotocamera); da un **motore
   web** dentro l'app (Electron, Chromium, CEF, WebKit, Qt WebEngine,
   Firefox: le pagine possono fare chiamate → audio, fotocamera,
   notifiche), anche quando si porta tutto da sé come Discord; dalle
   categorie (chat e chiamate → audio, fotocamera, notifiche; audio e
   video → audio; browser → Download); dai tipi di file che apre
   (immagini → Immagini, PDF e testo → Documenti, ...); e da quello che
   i programmi nominano (`/dev/video`, il portal
   Camera → fotocamera; `libpulse.so`, `libpipewire`, `libasound.so` →
   audio). L'autore spunta o toglie quello che vuole. L'ultima voce,
   **"Add a custom permission…"**, chiede tipo, nome e motivo di un
   permesso personalizzato (sezione 10) e lo aggiunge spuntato. Da riga di
   comando: `--permission camera`, `--permission "camera:Per le
   videochiamate"`, `--permission "folder:~/Giochi:ro=Per i salvataggi"`
   (ripetibile; quelli personalizzati vogliono il motivo dopo `=`);
4. le **euristiche** qui sotto ("Cosa propone il wizard");
5. scrive `<pacchetto>/recipe.json` (`--output` per un'altra cartella);
6. chiede se costruire subito l'app e metterla in `~/Applications`
   (`--no-build` per fermarsi alla ricetta).

#### Cosa propone il wizard

Il wizard non conosce le singole app: guarda solo quello che dicono i
file (`scan.sh`), **propone**, l'utente conferma, e tutto finisce scritto
nella ricetta, dove si vede e si cambia. `xapp build` poi fa solo quello
che c'è nella ricetta.

| Cosa trova nei file | Cosa propone |
|---|---|
| librerie **nominate** nei programmi e caricate durante l'uso (`dlopen`), che le dipendenze non portano (per esempio `libpulse.so.0` in Electron, una dipendenza solo opzionale del pacchetto) | i pacchetti che le contengono, con una domanda (tutte le sorgenti: `--arch`, `--aur`, `--url`, `--dir`) |
| le **dipendenze opzionali** del pacchetto (`--arch`, `--aur`): quello che chi lo impacchetta dice che può usare, anche quando i file non lo dicono (un programma che l'app scarica all'avvio) | una lista a caselle; già spuntate quelle che, dalla descrizione, riguardano suono, fotocamera, notifiche, tray, grafica (Discord: `libpulse`, "Pulseaudio support") |
| un pacchetto, o una sua dipendenza, del repository **multilib**; oppure programmi ELF **a 32 bit** | `"multilib": true` (e, per archivi e cartelle, i `lib32-*` con le librerie che i programmi a 32 bit chiedono) |
| programmi a 32 bit che disegnano con OpenGL o Vulkan (o un loader `lib32-` nella chiusura) | i driver grafici a 32 bit (`lib32-mesa`, `lib32-vulkan-radeon`, `-intel`, `-nouveau`): XeeonOS non ne ha da prestare |
| il permesso `audio`, ma solo ALSA (`alsa-lib` senza librerie di PulseAudio o PipeWire) | `pipewire-alsa`: nella sandbox non c'è la scheda audio, solo il socket di PipeWire |
| quello che i programmi nominano (`/dev/video`, il portal Camera; `libpulse.so`, `libpipewire`, `libasound.so`) | i permessi `camera`, `audio` già spuntati nella lista |
| **archivi dentro l'app** (tar, zip; più di 100 KiB): launcher che si scompattano all'avvio, come il bootstrap di Steam | vengono aperti e guardati come gli altri file: librerie che chiedono e nominano, programmi a 32 bit, permessi. Anche `xapp build` li apre: programmi a 32 bit → `multilib` nel manifest; un programma che si collega a X11 e non a Wayland → la capability `x11`; i segni di una sandbox propria (Chromium, CEF...) → `user_namespaces` |
| dipendenze **virtuali** (`vulkan-driver`, `ttf-font`, `java-runtime`...) per cui pacman sceglierebbe da solo (lette dai database dello snapshot: cosa fornisce ogni pacchetto, contro quello che pacman installerebbe; pacman non chiede quando stampa soltanto) | una domanda per ognuna, con prima un pacchetto che l'app ha già o, se no, uno per hardware che XeeonOS supporta (mai NVIDIA, CUDA, ROCm o GPU di telefoni per primi); il nome scelto va nella ricetta |
| pacchetti (della chiusura o opzionali) con **solo file per il sistema**: regole udev (`steam-devices`), moduli del kernel (`ntsync-autoload`), servizi o polkit | una nota nel riepilogo: dentro un'app non fanno niente, deve averli XeeonOS. Gli opzionali così non vengono proposti |
| programmi di `base` (`tar`, `sed`, `grep`, `gzip`, `findutils`...) che l'app non ha: per regola di Arch i pacchetti non li nominano, ma gli script di avvio li chiamano (Steam si scompatta con `tar`) | quelli mancanti, con una domanda (già sì); mai systemd, pacman o il kernel |

Con `--url` (solo `https://`):

1. nel container scarica l'archivio (tar.xz, tar.gz, tar.bz2, zip...),
   segue i redirect e ne calcola lo SHA-256: nella ricetta vanno
   l'indirizzo **finale** (es. quello con il numero di versione al posto di
   "latest") e lo SHA-256, così la build prende sempre lo stesso file;
2. trova i programmi (ELF con interprete) e con `readelf` le librerie che
   chiedono; quelle che l'archivio non ha le cerca nel database dei file
   dello snapshot (`pacman -F`) e mette i loro pacchetti nella ricetta. Le
   librerie nominate dentro i programmi e caricate solo durante l'uso
   (`dlopen`: audio, video, accelerazione, stampa) le propone con una
   domanda a parte. Quelle che nessun pacchetto ha vengono mostrate
   (l'app potrebbe non partire);
3. nome e versione vengono dal nome del file (`firefox-147.0.tar.xz`),
   l'id dall'indirizzo scritto (`download.mozilla.org` → `org.mozilla.firefox`;
   `github.com/<autore>` → `io.github.<autore>.<nome>`; `--id` per
   sceglierlo); il programma da avviare è quello con il nome dell'app (o
   quello del `.desktop` dell'archivio, se c'è; `--exec <percorso>`
   altrimenti); l'icona è l'immagine più grande in una cartella o file
   "icon" (`--icon <percorso>` altrimenti);
4. l'archivio finisce in `/opt/<nome>` dell'immagine (`sources`); i
   permessi sono proposti come per `--arch`, dalle librerie trovate.

Con `--dir <cartella>` (un programma compilato in casa, un archivio già
scompattato): come `--url`, senza download. La cartella è montata in sola
lettura nel container; nome e versione vengono dal suo nome, l'id è
`org.xeeonos.app.<nome>` (`--id` per sceglierlo). Nella ricetta la
sorgente è la cartella, con il percorso assoluto: per conservare la
ricetta, o metterla in xeeonos-applications, la cartella va dentro la
ricetta e il percorso diventa relativo.

Con `--aur <pacchetto>`:

1. nel container scarica il repository AUR del pacchetto (l'ultimo
   commit) e controlla che quello che serve per costruirlo e per usarlo
   sia nello snapshot. Se manca qualcosa (di solito altri pacchetti AUR,
   non ancora gestiti, o versioni più nuove: `--snapshot`) si ferma e lo
   dice;
2. **mostra il PKGBUILD** e chiede se costruirlo: è codice scritto dal
   manutentore, non controllato da Arch. La risposta predefinita è "Non
   costruire", e `--yes` da solo non lo costruisce mai; `--pkgbuild-ok`
   lo costruisce senza chiedere (script che lo leggono in altro modo);
3. lo costruisce con `makepkg`, come utente normale e mai come root, in una
   radice fatta dallo snapshot (le stesse librerie che avrà l'app), dentro
   il container: mai sul sistema;
4. da lì è come `--arch`: voce di menu, icona, permessi proposti. Nella
   ricetta il pacchetto va in `aur` con il **commit** del PKGBUILD letto,
   così `xapp build` costruisce sempre quello.

`--yes` accetta tutte le proposte (per la CI). `user_namespaces` e le
capacità non compaiono: le decide `xapp build` (sezioni 8 e 10).

## 15. Il comando xapp

```
xapp run [--yes] [--trace] <file|app> [argomenti]
                                          avvia (anche quello che fa il doppio clic);
                                          --trace: nota cosa la sandbox rifiuta
xapp install [--yes] <file>               copia in ~/Applications
xapp build <ricetta> [--key <file>] [--output <file>] [--no-install] [--root <dir>]
xapp create --arch <pacchetto> | --aur <pacchetto> | --url <https://...> | --dir <cartella>
            [--exec <programma>] [--icon <immagine>] [--pkgbuild-ok]
            [--permission <id>[=<motivo>]]... [--snapshot AAAA/MM/GG] [--id <id>] [--output <dir>] [--no-build]
                                          wizard: ricetta da un pacchetto Arch o AUR, da un
                                          archivio o da una cartella, poi build
xapp list                                 le applicazioni di ~/Applications
xapp info <app|file>                      cos'è, autore, snapshot, permessi
xapp verify <file>                        firma e integrità, senza aprire
xapp remove <app> [--purge]               toglie il file (i dati restano; --purge: anche quelli)
xapp permissions <app> [<permesso> yes|no|ask | --reset]
                                          mostra o cambia i permessi (anche uno non
                                          dichiarato); --reset: dimentica le scelte
xapp permissions --all [<permesso> yes|no|ask]
                                          lo stesso per tutte le app
xapp doctor <app> [--reset]               permette quello che --trace ha visto rifiutare,
                                          e trova i pacchetti di quello che mancava
xapp logs <app> [-f]                      i messaggi dell'app (journal)
xapp ps                                   le app aperte: pid, da quanto, memoria
xapp kill <app>                           chiude l'app (tutte le sue finestre e processi)
```

`xapp ps` legge le cartelle delle istanze (`$XDG_RUNTIME_DIR/xapp/run/<id>-<pid>`)
e la memoria dallo scope systemd di ciascuna. `xapp kill` ferma lo scope
(tutti i processi dell'app); senza il gestore dei servizi dell'utente
termina `xapp run`, e bubblewrap muore con lui.

Interni, usati dal sistema: `xapp sync` (monitor), `xapp dialog <file>`
(finestra dei dialoghi), `xapp thumbnail <file> <png> [dim]` (miniature).

Non c'è nessun repository o store: l'applicazione si scarica da internet
come su macOS, poi si apre o si installa. `xapp` è un comando a sé, non è
dentro xctl, e non richiede sudo.

## 16. Il codice

`extensions/xapp/source/` è un workspace Cargo con crate separate, come xctl:

| Crate | Compito |
|---|---|
| `xapp-core` | errori, programmi esterni, percorsi, dialoghi (`ui.rs`), versioni, date |
| `xapp-format` | il file: manifest e contenitore (immagine + trailer) |
| `xapp-trust` | chiavi Ed25519, firma, autori fidati, pinning, revoca |
| `xapp-permissions` | catalogo dei permessi e delle capacità, scelte dell'utente (per app e per tutte), permessi aggiunti a mano, aggiunte di `xapp doctor` (`overrides.rs`) |
| `xapp-sandbox` | montaggio, seccomp, filtro D-Bus, GPU, riga di comando di bubblewrap, avvio nello scope, `--trace` (`trace.rs`), app aperte (`instances.rs`) |
| `xapp-desktop` | registro, voci di menu, tipo di file, `xapp sync` |
| `xapp-build` | `xapp build` (con `build-root.sh` per il container) e il wizard `xapp create` (`create.rs`, `create-arch.sh`, `create-url.sh`, `create-aur.sh`, quello che dicono i file in `scan.sh`), i pacchetti AUR (`aur.sh`) |
| `xapp` | il programma: legge la riga di comando |

Dipendenze esterne, poche e note: `serde_json` (JSON), `sha2` (SHA-256),
`ed25519-dalek` (firme). Cargo.lock le fissa e la build usa `--locked`.
Il binario viene compilato dentro l'estensione (`extensions/xapp/build.sh`,
con il Rust dello stesso snapshot, dopo `cargo fmt --check`, `cargo clippy`
e `cargo test`: se uno fallisce, l'estensione non si costruisce). Programmi del sistema usati:
`bwrap`, `erofsfuse`/`fusermount3`, `xdg-dbus-proxy`, `systemd-run`,
`whiptail`, `update-mime-database`, per le miniature SVG `rsvg-convert`,
e per `xapp build` `podman` o `docker`.

## 17. Test

- Unitari (`cargo test` in `extensions/xapp/build.sh`): file
  `.xapp` scritto e riletto, un byte cambiato rifiutato, manifest,
  firma, filtro seccomp eseguito da un piccolo interprete BPF (con e senza
  sandbox propria), riconoscimento di Chromium, Electron e WebKitGTK, delle
  interfacce solo X11, regole del bus (con le aggiunte a mano), lettura del
  log del proxy, resoconto di `--trace`, finto server X11, sandbox del proxy, `/.xapp-info`,
  prestito dei file, mimeapps.list, versioni e date.
- In VM (`extensions/xapp/test`, eseguito da `tools/boot-check.sh` sulla
  versione di XeeonOS pubblicata): un'app di prova costruita nella VM con
  `xapp build --root` (bash e pochi comandi del sistema), messa in
  `~/Applications` durante una sessione GNOME. Si verificano: il menu, il
  tipo di file anche senza estensione, il doppio clic; nella sandbox:
  immagine propria, sistema e home invisibili, cartella permessa sì e negata
  no, niente rete, niente namespace utente né `bwrap` dentro la sandbox;
  una seconda app con un file di Chromium, a cui `xapp build` dà la sandbox
  propria senza chiedere niente, dove namespace utente e `bwrap` funzionano;
  `/usr` in sola lettura, Wayland, driver grafici del sistema prestati, bus
  filtrato (portal sì, GNOME Shell no), file aperto prestato; il portal
  dell'estensione al posto di quello del sistema, che riconosce l'app come
  xapp (il document portal risponde "Not allowed in sandbox"), la cartella
  `doc`, e i pulsanti della finestra impostati in GNOME letti dal portal. Vengono rifiutati un
  file cambiato, la copia di un altro autore e un'app revocata. Per un
  autore sconosciuto si vede la domanda, e senza nessuno a cui chiedere
  l'app non si apre. Infine miniatura, lista, info, e la rimozione che
  toglie la voce dal menu e lascia i dati.
- `xapp build` da una ricetta (pacchetti Arch in un container) e Firefox
  avviato nella sandbox: nella CI di
  [xeeonos-applications](https://github.com/ivansollazzo/xeeonos-applications).
- Il wizard da capo a fondo (workflow "xapp wizard", sulle pull request
  che toccano xapp): `--dir` con un programma del runner, la sua voce di
  menu e l'icona; `--aur` con `--yes` senza `--pkgbuild-ok` rifiutato;
  `--aur` di un'app vera (xclicker); ogni ricetta costruita fino al file
  `.xapp`, verificato.

## 18. Limiti noti e lavori futuri

- **Tipo "xapp" upstream** in xdg-desktop-portal, così da non dover
  sostituire il portal del sistema.
- **Microfono separato dalla riproduzione** (regole WirePlumber o portal).
- **Domande al momento dell'uso** invece che all'avvio.
- **Dialoghi grafici** invece che da terminale.
- **security-context-v1** di Wayland, se Mutter lo supporterà.
- **Chiave XeeonOS** per le app ufficiali di xeeonos-applications (la
  pubblica in `/usr/lib/xapp/trusted.d/`, la privata come segreto della
  CI), e rotazione delle chiavi.
- **Dipendenze AUR di pacchetti AUR**: oggi `--aur` si ferma e le
  elenca.
- **Deduplicazione** tra immagini, aggiornamenti dalla rete, app condivise
  tra utenti, app in GNOME Software: possibili senza cambiare il formato.
