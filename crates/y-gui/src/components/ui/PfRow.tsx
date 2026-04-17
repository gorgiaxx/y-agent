import { type ReactNode } from 'react'

type PfRowCols = 2 | 3 | 4 | '2-1'

interface PfRowProps {
  cols?: PfRowCols
  className?: string
  children: ReactNode
}

const colsClassMap: Record<PfRowCols, string> = {
  2: 'pf-row',
  3: 'pf-row pf-row-triple',
  4: 'pf-row pf-row-quad',
  '2-1': 'pf-row pf-row-2-1',
}

export function PfRow({ cols = 2, className = '', children }: PfRowProps) {
  return (
    <div className={[colsClassMap[cols], className].filter(Boolean).join(' ')}>
      {children}
    </div>
  )
}
