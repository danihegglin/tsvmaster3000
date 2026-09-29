# Renders docs/demo.gif for the README.
#   cargo build --release && vhs docs/demo.tape

Output docs/demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 560
Set Padding 20
Set Theme "TokyoNight"
Set TypingSpeed 90ms

Hide
Type "export PS1='$ ' && clear" Enter
Show

Type "tsv samples/people.tsv" Sleep 400ms
Hide
# Run the freshly built binary without printing its path.
Ctrl+U Type "clear && target/release/tsv samples/people.tsv" Enter
Show
Sleep 1.5s

# Move around.
Type "jjj" Sleep 300ms
Type "ll" Sleep 800ms

# Edit a cell.
Type "a" Sleep 400ms
Type ", CH" Sleep 400ms
Escape Sleep 1s

# Add a column to the right and give it a header.
Type "g" Sleep 500ms
Type "tt" Sleep 1s
Type "i" Type "country" Escape Sleep 1s

# Delete a column, then undo and redo it.
Type "l" Sleep 400ms
Type "xx" Sleep 1.5s
Type "u" Sleep 1.5s
Ctrl+R Sleep 1.5s
Type "u" Sleep 1s

# Undo everything, then leave without saving.
Type "uuu" Sleep 1.5s
Type ":q!" Sleep 600ms
Enter
Sleep 500ms
