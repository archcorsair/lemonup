Source tapes/common.tape
Output shots/backup.txt

# Launch hidden; the addon list scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# b opens Backups. Inside it, b would back up now, so nothing else is pressed.
Type "b"
Show
Sleep 1500ms
Screenshot shots/backup.png

Hide
Escape
Type "q"
Sleep 1s
