Source tapes/common.tape
# Light terminal, so default-foreground text stays readable on the light UI.
Set Theme "tokyonight-day"
Output shots/light.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# generate.sh switches the sandbox config to theme = "light" for this tape.
Down
Show
Sleep 1500ms
Screenshot shots/light.png

Hide
Type "q"
Sleep 1s
