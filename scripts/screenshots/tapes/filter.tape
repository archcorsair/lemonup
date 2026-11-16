Source tapes/common.tape
Output shots/filter.txt

# Launch hidden; the addon list scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# / opens the filter box over the header. "options" matches three addons
# through the *Options folders that belong to them.
Type "/"
Sleep 300ms
Type "options"
Sleep 300ms
# enter keeps the filter; the box stays in place of the header.
Enter
Show
Sleep 1500ms
Screenshot shots/filter.png

Hide
Escape
Type "q"
Sleep 1s
