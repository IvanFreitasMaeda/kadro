# Kadro

Captura de tela simples e rápida para macOS e Windows, inspirada no Lightshot e no CleanShot.

Feito com [Tauri 2](https://v2.tauri.app) (Rust + TypeScript), para ser leve: o instalador fica na casa de poucos MB.

## Download

Baixe a versão mais recente na página de [Releases](https://github.com/IvanFreitasMaeda/kadro/releases/latest):

| Sistema | Download |
| --- | --- |
| macOS com Apple Silicon (M1 ou mais novo), macOS 12.3+ | [Kadro-macOS-AppleSilicon.dmg](https://github.com/IvanFreitasMaeda/kadro/releases/latest/download/Kadro-macOS-AppleSilicon.dmg) |
| Windows 10 (versão 2004+) ou 11 | [Kadro-Windows-Setup.exe](https://github.com/IvanFreitasMaeda/kadro/releases/latest/download/Kadro-Windows-Setup.exe) · [.msi](https://github.com/IvanFreitasMaeda/kadro/releases/latest/download/Kadro-Windows.msi) |

Macs com processador Intel não são suportados. Depois de baixar, veja [Instalar no macOS](#instalar-no-macos) ou [Instalar no Windows](#instalar-no-windows): no Mac são dois comandos no Terminal, porque o app não tem assinatura da Apple.

## O que faz

- Fica na barra de menus (macOS) ou na bandeja (Windows).
- **Um clique** no ícone abre a captura. Clique direito mostra o menu.
- Selecione uma área arrastando, ou pegue a tela inteira.
- Ajuste a seleção depois: arraste para mover, use as alças para redimensionar.
- Anote antes de copiar: lápis, marca-texto, linha, seta, retângulo, elipse e texto, com cores, três espessuras e desfazer/refazer.
- Copie para a área de transferência ou salve em PNG numa pasta.
- Histórico na lateral: as últimas capturas aparecem em miniatura, e um clique copia de novo. Dá para salvar, abrir ou remover cada uma dali.
- Grave a tela inteira ou uma área em **MP4** ou **GIF**. Uma barrinha mostra o tempo e o botão de parar, e fica de fora do vídeo. A gravação vai para a pasta de capturas e aparece no histórico.
- Atalhos, pasta de capturas e tamanho do histórico configuráveis em Preferências.
- Funciona com vários monitores.

## Atalhos

| Ação | macOS | Windows |
| --- | --- | --- |
| Capturar área | `⌘ ⇧ 2` | `Ctrl Shift 2` |
| Capturar tela inteira | `⌘ ⇧ 1` | `Ctrl Shift 1` |
| Abrir/fechar o histórico | `⌘ ⇧ 0` | `Ctrl Shift 0` |
| Gravar a tela / parar a gravação | `⌘ ⇧ 9` | `Ctrl Shift 9` |

Dá para trocar os atalhos em **Preferências…**, no menu do ícone: clique no atalho e pressione a nova combinação.

Dentro da captura:

| Tecla | Ação |
| --- | --- |
| `Enter`, `Ctrl/⌘ C` ou duplo clique | Copiar e fechar |
| `Ctrl/⌘ S` | Salvar na pasta de capturas |
| `Ctrl/⌘ Shift S` | Salvar como… |
| `F` ou `Ctrl/⌘ A` | Selecionar a tela inteira |
| `Esc` ou clique direito | Cancelar |
| `P` `M` `L` `A` `R` `E` `T` | Lápis, marca-texto, linha, seta, retângulo, elipse, texto (de novo para soltar) |
| `Shift` enquanto desenha | Trava o ângulo da linha/seta, faz quadrado ou círculo |
| `Ctrl/⌘ Z` · `Ctrl/⌘ Shift Z` | Desfazer · refazer |

No texto, `Enter` confirma e `Shift Enter` quebra a linha.

Na gravação:

| Tecla | Ação |
| --- | --- |
| Arrastar | Escolher a área (ou `F` para a tela inteira) |
| `V` · `G` | Vídeo MP4 · GIF animado |
| `Enter` ou duplo clique | Começar a gravar |
| `⌘ ⇧ 9` / `Ctrl Shift 9`, ou clique no ícone do Kadro | Parar e salvar |

O botão de lixeira na barrinha descarta a gravação (clique duas vezes). O MP4 sai em 30 quadros por segundo, na resolução da tela (até 4K). O GIF sai em 15 quadros por segundo e no tamanho "normal" da tela (sem o dobro do Retina), com no máximo 1280 px de largura, para não ficar pesado.

As capturas são salvas em `Imagens/Kadro` por padrão. Dá para trocar a pasta em **Preferências…**.

No histórico:

| Tecla | Ação |
| --- | --- |
| Clique na miniatura ou `Enter` | Copiar (gravações são copiadas como arquivo, para colar num chat ou pasta) |
| `↑` `↓` | Escolher a captura |
| `Ctrl/⌘ S` · `Ctrl/⌘ Shift S` | Salvar na pasta (gravações: mostrar na pasta) · salvar como… |
| `Espaço` | Abrir no visualizador de imagens ou player |
| `Delete` | Remover do histórico |
| `Esc` ou clique fora | Fechar |

O histórico guarda as últimas 30 capturas por padrão (dá para mudar ou desativar em **Preferências…**). Elas ficam na pasta de dados do app, separadas da pasta de capturas: remover do histórico não apaga o que você salvou. As gravações não são duplicadas: o histórico só aponta para o arquivo na pasta de capturas.

## Instalar no macOS

1. Abra o `Kadro-macOS-AppleSilicon.dmg` e arraste o Kadro para **Aplicativos**.
2. O app não tem assinatura da Apple, então o Mac bloqueia na primeira vez. Rode no Terminal:

   ```sh
   xattr -cr /Applications/Kadro.app
   tccutil reset ScreenCapture io.github.ivanfreitasmaeda.kadro
   ```

   O primeiro comando tira a "quarentena" do download, que faz o macOS dizer que o app está danificado. O segundo limpa a permissão de gravação de tela de uma versão anterior, se houver.
3. Abra o Kadro. Ele não abre janela: aparece como um ícone na barra de menus. Se ainda aparecer aviso, vá em **Ajustes do Sistema → Privacidade e Segurança** e clique em **Abrir Mesmo Assim**.
4. Na primeira captura o macOS pede a permissão de **Gravação de Tela**. Depois de liberar, feche o Kadro pelo menu do ícone e abra de novo.

Enquanto o app não tiver assinatura da Apple, cada versão nova é vista pelo macOS como outro app. Se a permissão aparecer marcada mas a captura não funcionar, rode de novo o `tccutil reset` acima, abra o Kadro e libere outra vez.

## Instalar no Windows

Rode o `Kadro-Windows-Setup.exe` (ou o `.msi`, se preferir). O SmartScreen pode avisar que o app é desconhecido: clique em **Mais informações → Executar assim mesmo**. O Kadro aparece como um ícone na bandeja, perto do relógio.

## Atualizar ou desinstalar

Para instalar uma versão nova do zero, apague a anterior e os dados dela antes. Isso também apaga o histórico e as preferências. As capturas que você salvou na pasta `Imagens/Kadro` **não** são apagadas.

**macOS** (no Terminal):

```sh
osascript -e 'quit app "Kadro"' 2>/dev/null
rm -rf /Applications/Kadro.app \
  ~/Library/Application\ Support/io.github.ivanfreitasmaeda.kadro \
  ~/Library/Caches/io.github.ivanfreitasmaeda.kadro \
  ~/Library/WebKit/io.github.ivanfreitasmaeda.kadro
tccutil reset ScreenCapture io.github.ivanfreitasmaeda.kadro
```

Depois é só seguir o [Instalar no macOS](#instalar-no-macos) com o `.dmg` novo.

**Windows:** feche o Kadro pelo menu do ícone e desinstale em **Configurações → Aplicativos → Kadro → Desinstalar**. Depois, para apagar os dados, rode no PowerShell:

```powershell
Remove-Item -Recurse -Force "$env:APPDATA\io.github.ivanfreitasmaeda.kadro", "$env:LOCALAPPDATA\io.github.ivanfreitasmaeda.kadro" -ErrorAction SilentlyContinue
```

## Desenvolvimento

Requisitos: [Node.js 22+](https://nodejs.org), [Rust](https://rustup.rs) e os [pré-requisitos do Tauri](https://v2.tauri.app/start/prerequisites/) do seu sistema.

```sh
npm install
npm run tauri dev     # roda em modo desenvolvimento
npm run tauri build   # gera o instalador em src-tauri/target/release/bundle
```

Para publicar uma versão nova: suba o número da versão em `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` e `package.json`, faça o merge na `main` e crie a tag (`git tag v0.2.0 && git push origin v0.2.0`). O workflow `Release` gera os instaladores do Mac e do Windows e publica a release com eles. A cada PR o workflow `Build` também gera os instaladores, na aba **Actions**, para teste.

Estrutura:

- `src-tauri/src/capture.rs`: captura dos monitores, janelas de seleção, recorte, copiar e salvar.
- `src-tauri/src/tray.rs`: ícone e menu da bandeja.
- `src-tauri/src/shortcuts.rs`: atalhos globais configuráveis.
- `src-tauri/src/history.rs`: histórico de capturas (imagem ou gravação, e miniatura em disco).
- `src-tauri/src/record/`: gravação de tela. `macos.rs` usa ScreenCaptureKit + AVAssetWriter, `windows.rs` usa Windows Graphics Capture + Media Foundation (os dois codificam H.264 com o codificador do próprio sistema), `gif.rs` monta o GIF.
- `src-tauri/src/fileclip.rs`: copia um arquivo (a gravação) para a área de transferência.
- `src-tauri/src/gallery.rs`: janela lateral do histórico.
- `src-tauri/src/settings.rs`: janela de preferências.
- `src-tauri/src/config.rs`: configurações salvas (atalhos, pasta de capturas e tamanho do histórico).
- `src/overlay.ts`: interface de seleção e anotação.
- `src/annotate.ts`: formas das anotações e como desenhá-las.
- `src/gallery.ts`: interface do histórico.
- `src/record.ts`: barrinha de controle da gravação.
- `src/settings.ts`: interface das preferências.

## Créditos

A ideia e os testes são do Ivan Freitas Maeda. O desenvolvimento foi feito com a ajuda do [Claude](https://claude.ai), da Anthropic, para agilizar o trabalho.

## Licença

MIT
