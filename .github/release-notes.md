## Downloads

| Sistema | Arquivo |
| --- | --- |
| macOS (Apple Silicon, M1 ou mais novo, macOS 12.3+) | `Kadro-macOS-AppleSilicon.dmg` |
| Windows 10 (2004+) / 11, instalador | `Kadro-Windows-Setup.exe` |
| Windows, pacote MSI | `Kadro-Windows.msi` |
| Linux (beta), Ubuntu/Debian/Mint | `Kadro-Linux-x86_64.deb` (ARM: `Kadro-Linux-arm64.deb`) |
| Linux (beta), Fedora/openSUSE | `Kadro-Linux-x86_64.rpm` (ARM: `Kadro-Linux-arm64.rpm`) |
| Linux (beta), qualquer distribuição | `Kadro-Linux-x86_64.AppImage` (ARM: `Kadro-Linux-arm64.AppImage`) |
| Arch Linux | AUR: `yay -S kadro-bin` |

Macs com processador Intel não são suportados.

**macOS:** o app não tem assinatura da Apple. Depois de arrastar o Kadro para **Aplicativos**, rode no Terminal:

```sh
xattr -cr /Applications/Kadro.app
tccutil reset ScreenCapture io.github.ivanfreitasmaeda.kadro
```

**Windows:** se o SmartScreen avisar, clique em **Mais informações → Executar assim mesmo**.

**Linux:** `sudo apt install ./Kadro-Linux-x86_64.deb`, `sudo dnf install ./Kadro-Linux-x86_64.rpm`, ou `chmod +x` no AppImage e abra. No Wayland, os atalhos podem precisar ser criados nas configurações de teclado do sistema (`kadro --area` etc.): as Preferências do Kadro mostram como.

Instruções completas, inclusive como limpar uma versão anterior, no [README](https://github.com/IvanFreitasMaeda/kadro#download).
