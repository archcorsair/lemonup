Source tapes/common.tape
Output shots/dashboard.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# Cursor on Deadly Boss Mods: update pending, owned modules.
Down
Show
Sleep 1500ms
Screenshot shots/dashboard.png

Hide
Type "q"
Sleep 1s
