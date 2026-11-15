FOSSPS @VERSION@
Free, open-source engine for multiplayer first-person shooters.
https://github.com/tahuffman1s/FOSSPS

In this folder
  fossps          makes, plays, checks, hosts and packages games
  fossps-server   hosts a game on a machine with no screen or graphics card
  licenses/       licences of the code built into both programs

Start here (in a terminal opened in this folder; on Windows leave out "./")
  ./fossps new my-game      creates a game in the folder my-game
  ./fossps run my-game      plays it
  ./fossps check my-game    tests its script without opening a window
  ./fossps --help           lists everything else

To use "fossps" from any folder, add this folder to your PATH, or move the
two programs into a folder that is already on it.

If it does not start
  "Vulkan is missing or too old": update your graphics driver. FOSSPS needs
  Vulkan 1.3. On Linux, also install your distribution's Vulkan loader
  (for example: sudo dnf install vulkan-loader, sudo apt install libvulkan1).

  macOS says the program "cannot be opened" or is "damaged": these builds are
  not signed by Apple yet. Run this once in this folder:
      xattr -dr com.apple.quarantine .
  macOS also needs MoltenVK for Vulkan (brew install molten-vk), and support
  for macOS is still experimental.

  Windows SmartScreen says "Windows protected your PC": choose "More info",
  then "Run anyway". These builds are not signed yet.

Something else wrong, or an idea?
  https://github.com/tahuffman1s/FOSSPS/issues

FOSSPS is licensed under MIT or Apache-2.0, at your choice (LICENSE-MIT,
LICENSE-APACHE).
