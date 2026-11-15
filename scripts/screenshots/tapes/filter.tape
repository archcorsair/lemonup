Source tapes/common.tape
Output shots/filter.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
Type "/"
Sleep 300ms
# "options" matches three addons through their owned *Options folders.
Type "options"
Sleep 300ms
Enter
Show
Sleep 1500ms
Screenshot shots/filter.png

Hide
Escape
Type "q"
Sleep 1s
